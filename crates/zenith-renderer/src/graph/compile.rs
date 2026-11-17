//! From a `GraphDesc` to a `CompiledGraph`: checks, resource versions, the
//! order of the passes, lifetimes.
//!
//! 1. The passes are made by their type; a node of an unknown type is left out
//!    with its links.
//! 2. The links are checked (the slots exist, the kinds match, one link an
//!    input). A pass without a required input is left out, and so on down the
//!    chain.
//! 3. The passes no output node depends on are left out. No output node left:
//!    the graph is broken.
//! 4. Resources: an output creates a texture, publishes data, or passes an
//!    input on. Drawing into an input (`Access::Write`) makes a new version of
//!    its texture; the passes that read the old version must come first.
//! 5. The order: the links and those edges, sorted with ties broken by node id,
//!    so it is the same every time. A cycle is an error.
//! 6. Textures with the same description whose lifetimes do not overlap share
//!    one physical texture.

use std::{
	any::Any,
	collections::{BTreeMap, BTreeSet, HashMap},
	sync::Arc,
};

use super::{
	desc::{GraphDesc, NodeDesc, NodeId, SlotRef},
	exec::{BindGroupCache, ViewKey},
	pass::{
		Access, FormatRule, OutputSource, ParamMap, ParamValue, PassCategory, PassDesc, PassRegistry, RenderPass,
		SizeRule,
	},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
	/// The graph works without what it is about (a node left out).
	Warning,
	/// The graph does not compile.
	Error,
}

/// What compiling found, on a node or on the whole graph: the editor shows it
/// on the node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
	pub node: Option<NodeId>,
	pub severity: Severity,
	pub message: String,
	/// Other nodes it is about (the nodes of a cycle, in order), for the
	/// editor to mark too.
	pub related: Vec<NodeId>,
}

impl std::fmt::Display for Diagnostic {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self.node {
			Some(node) => write!(f, "{node}: {}", self.message),
			None => f.write_str(&self.message),
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ResourceId(pub usize);

/// A number of pixels from a size that may be scaled: rounded, at least 1.
#[allow(clippy::cast_sign_loss)] // clamped to 1 and more first
pub(crate) const fn pixels(value: f32) -> u32 {
	value.round().max(1.0) as u32
}

/// The size of a texture once the parameters are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ResolvedSize {
	/// A share of the view, as the bits of an `f32` (so it can be a key).
	View(u32),
	Absolute(u32, u32),
}

impl ResolvedSize {
	/// In pixels, for a view of `size`; at least 1 a side.
	pub(crate) fn pixels(self, (width, height): (u32, u32)) -> (u32, u32) {
		match self {
			Self::View(bits) => {
				let scale = f32::from_bits(bits);
				(pixels(width as f32 * scale), pixels(height as f32 * scale))
			}
			Self::Absolute(width, height) => (width.max(1), height.max(1)),
		}
	}
}

pub(crate) enum ResourceKind {
	Texture {
		format: FormatRule,
		size: ResolvedSize,
		/// The texture it is drawn into; `None`: the target of the view.
		physical: Option<usize>,
	},
	Data,
}

pub(crate) struct Resource {
	pub kind: ResourceKind,
	/// Where it is made, for messages: `#3.color`.
	pub name: String,
}

impl Resource {
	pub(crate) const fn is_view_target(&self) -> bool {
		matches!(
			self.kind,
			ResourceKind::Texture {
				format: FormatRule::ViewTarget,
				..
			}
		)
	}
}

/// What a slot of a node is bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
	pub resource: ResourceId,
	/// The node is the first to draw into it: its render pass clears it.
	pub clear: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeState {
	/// `init` has not run yet.
	New,
	Ready,
	/// `init` failed: it is left out of every frame.
	Broken,
}

pub(crate) struct CompiledNode {
	pub id: NodeId,
	pub label: String,
	pub pass: Box<dyn RenderPass>,
	pub params: ParamMap,
	pub inputs: HashMap<String, Binding>,
	pub outputs: HashMap<String, Binding>,
	pub state: NodeState,
	/// An error of `exec` was logged already.
	pub reported: bool,
	pub bind_groups: BindGroupCache,
	pub view_states: HashMap<ViewKey, Box<dyn Any + Send + Sync>>,
}

/// A graph ready to run: the passes in their order, the resources, the
/// physical textures.
pub struct CompiledGraph {
	pub(crate) name: String,
	pub(crate) nodes: Vec<CompiledNode>,
	pub(crate) resources: Vec<Resource>,
	/// The format and size of each physical texture.
	pub(crate) physical: Vec<(FormatRule, ResolvedSize)>,
	diagnostics: Vec<Diagnostic>,
}

impl CompiledGraph {
	pub fn name(&self) -> &str {
		&self.name
	}

	/// The nodes that run, in their order.
	pub fn order(&self) -> Vec<NodeId> {
		self.nodes.iter().map(|node| node.id).collect()
	}

	/// The warnings of compiling (there are no errors in a compiled graph).
	pub fn diagnostics(&self) -> &[Diagnostic] {
		&self.diagnostics
	}

	/// How many textures of its own a view of this graph needs.
	pub const fn physical_textures(&self) -> usize {
		self.physical.len()
	}

	/// The parameters of a node.
	pub fn params(&self, node: NodeId) -> Option<&ParamMap> {
		self.nodes.iter().find(|n| n.id == node).map(|n| &n.params)
	}
}

impl std::fmt::Debug for CompiledGraph {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("CompiledGraph")
			.field("name", &self.name)
			.field("order", &self.order())
			.field("physical", &self.physical.len())
			.finish_non_exhaustive()
	}
}

/// Compiles `desc` with the passes of `passes`.
///
/// # Errors
/// What was found, errors and warnings, when the graph cannot run.
pub fn compile(desc: &GraphDesc, passes: &PassRegistry) -> Result<CompiledGraph, Vec<Diagnostic>> {
	let mut c = Compiler::new(passes);
	c.make_nodes(desc);
	c.check_links(desc);
	c.drop_unlinked();
	if !c.keep_live() {
		return Err(c.diagnostics);
	}
	c.resolve_params();
	let link_edges: BTreeSet<(NodeId, NodeId)> = c.incoming.iter().map(|((to, _), from)| (from.0, *to)).collect();
	let Some(link_order) = c.sort(&link_edges, "the links make a cycle through") else {
		return Err(c.diagnostics);
	};
	for id in link_order {
		c.bind(id);
	}
	let Some(order) = c.order(link_edges) else {
		return Err(c.diagnostics);
	};
	c.check_outputs();
	if c.failed() {
		return Err(c.diagnostics);
	}
	let physical = c.place_textures(&order);
	let nodes = c.instances(order);
	if c.failed() {
		return Err(c.diagnostics);
	}
	Ok(CompiledGraph {
		name: desc.name.clone(),
		nodes,
		resources: c.resources,
		physical,
		diagnostics: c.diagnostics,
	})
}

/// A slot: a node and the name of one of its slots.
type Slot = (NodeId, String);

/// A version of a resource: a texture drawn into gets a new one.
type Version = (ResourceId, u32);

/// What the steps share.
struct Compiler<'a> {
	passes: &'a PassRegistry,
	diagnostics: Vec<Diagnostic>,
	nodes: BTreeMap<NodeId, (&'a NodeDesc, Arc<PassDesc>)>,
	/// The output linked to each input.
	incoming: BTreeMap<Slot, SlotRef>,
	params: BTreeMap<NodeId, ParamMap>,
	resources: Vec<Resource>,
	/// The node that creates each texture.
	created: HashMap<ResourceId, NodeId>,
	outputs: HashMap<Slot, Version>,
	inputs: HashMap<Slot, Version>,
	/// The node that draws over each version.
	writers: BTreeMap<Version, NodeId>,
	readers: BTreeMap<Version, Vec<NodeId>>,
}

impl<'a> Compiler<'a> {
	fn new(passes: &'a PassRegistry) -> Self {
		Self {
			passes,
			diagnostics: Vec::new(),
			nodes: BTreeMap::new(),
			incoming: BTreeMap::new(),
			params: BTreeMap::new(),
			resources: Vec::new(),
			created: HashMap::new(),
			outputs: HashMap::new(),
			inputs: HashMap::new(),
			writers: BTreeMap::new(),
			readers: BTreeMap::new(),
		}
	}

	fn warn(&mut self, node: Option<NodeId>, message: String) {
		self.diagnostics.push(Diagnostic {
			node,
			severity: Severity::Warning,
			message,
			related: Vec::new(),
		});
	}

	fn error(&mut self, node: Option<NodeId>, message: String) {
		self.diagnostics.push(Diagnostic {
			node,
			severity: Severity::Error,
			message,
			related: Vec::new(),
		});
	}

	fn failed(&self) -> bool {
		self.diagnostics.iter().any(|d| d.severity == Severity::Error)
	}

	/// Step 1: the passes of the nodes.
	fn make_nodes(&mut self, desc: &'a GraphDesc) {
		for node in &desc.nodes {
			if self.nodes.contains_key(&node.id) {
				self.error(Some(node.id), "two nodes have this id".into());
				continue;
			}
			match self.passes.get(&node.pass) {
				Some(pass) => {
					self.nodes.insert(node.id, (node, Arc::clone(pass)));
				}
				None => self.warn(
					Some(node.id),
					format!("there is no pass {:?}: the node is left out", node.pass),
				),
			}
		}
	}

	/// Step 2: the links that can be.
	fn check_links(&mut self, desc: &GraphDesc) {
		for link in &desc.links {
			let (Some((_, from)), Some((_, to))) = (self.nodes.get(&link.from.0), self.nodes.get(&link.to.0)) else {
				continue; // a node left out takes its links along
			};
			let problem = match (from.output_named(&link.from.1), to.input_named(&link.to.1)) {
				(None, _) => Some(format!("{} has no output {:?}", from.type_name, link.from.1)),
				(_, None) => Some(format!("{} has no input {:?}", to.type_name, link.to.1)),
				(Some(output), Some(input)) if output.kind != input.kind => Some(format!(
					"{} gives {:?} and {} takes {:?}",
					link.from, output.kind, link.to, input.kind
				)),
				_ => None,
			};
			if let Some(problem) = problem {
				self.warn(Some(link.to.0), format!("{problem}: the link is dropped"));
				continue;
			}
			let key = (link.to.0, link.to.1.clone());
			if self.incoming.contains_key(&key) {
				self.warn(
					Some(link.to.0),
					format!("{} is linked twice: the first link is used", link.to),
				);
				continue;
			}
			self.incoming.insert(key, link.from.clone());
		}
	}

	/// Step 2: the nodes without a required input go, and so on down the
	/// chain.
	fn drop_unlinked(&mut self) {
		loop {
			let missing: Vec<Slot> = self
				.nodes
				.iter()
				.filter_map(|(&id, (_, pass))| {
					pass.inputs
						.iter()
						.find(|input| {
							!input.optional
								&& !self
									.incoming
									.get(&(id, input.name.to_string()))
									.is_some_and(|from| self.nodes.contains_key(&from.0))
						})
						.map(|input| (id, input.name.to_string()))
				})
				.collect();
			if missing.is_empty() {
				break;
			}
			for (id, input) in missing {
				self.warn(
					Some(id),
					format!("the input {input:?} is not linked: the node is left out"),
				);
				self.nodes.remove(&id);
			}
		}
		let nodes = &self.nodes;
		self.incoming
			.retain(|(to, _), from| nodes.contains_key(to) && nodes.contains_key(&from.0));
	}

	/// Step 3: keeps what an output node depends on. `false`: there is no
	/// output node.
	fn keep_live(&mut self) -> bool {
		let mut live: BTreeSet<NodeId> = self
			.nodes
			.iter()
			.filter(|(_, (_, pass))| pass.category == PassCategory::Output)
			.map(|(&id, _)| id)
			.collect();
		if live.is_empty() {
			self.error(None, "no output node is left: the graph draws nothing".into());
			return false;
		}
		let mut stack: Vec<NodeId> = live.iter().copied().collect();
		while let Some(id) = stack.pop() {
			for ((to, _), from) in &self.incoming {
				if *to == id && live.insert(from.0) {
					stack.push(from.0);
				}
			}
		}
		self.nodes.retain(|id, _| live.contains(id));
		self.incoming.retain(|(to, _), _| live.contains(to));
		true
	}

	/// The parameters of the nodes (a texture may take its size from one).
	fn resolve_params(&mut self) {
		let mut warnings = Vec::new();
		for (&id, (node, pass)) in &self.nodes {
			let (values, found) = ParamMap::resolve(pass, &node.params);
			warnings.extend(found.into_iter().map(|warning| (id, warning)));
			self.params.insert(id, values);
		}
		for (id, warning) in warnings {
			self.warn(Some(id), warning);
		}
	}

	/// The nodes sorted along `edges`. `None`: a cycle, said with `message`.
	fn sort(&mut self, edges: &BTreeSet<(NodeId, NodeId)>, message: &str) -> Option<Vec<NodeId>> {
		match sort(self.nodes.keys().copied(), edges) {
			Ok(order) => Some(order),
			Err(stuck) => {
				let cycle = find_cycle(&stuck, edges);
				self.diagnostics.push(Diagnostic {
					node: cycle.first().copied(),
					severity: Severity::Error,
					message: format!("{message} {}", describe_cycle(&cycle)),
					related: cycle,
				});
				None
			}
		}
	}

	/// Step 4: the resources of the slots of `id` (its inputs are bound
	/// already: the nodes come in the order of the links).
	fn bind(&mut self, id: NodeId) {
		let Some(pass) = self.nodes.get(&id).map(|(_, pass)| Arc::clone(pass)) else {
			return;
		};
		// The version of each input once this pass has drawn into it.
		let mut after: HashMap<&str, Version> = HashMap::new();
		for input in &pass.inputs {
			let slot = (id, input.name.to_string());
			let Some(from) = self.incoming.get(&slot).cloned() else {
				continue;
			};
			let Some(&(resource, version)) = self.outputs.get(&(from.0, from.1.clone())) else {
				if !input.optional {
					self.error(Some(id), format!("{from} gives nothing to {:?}", input.name));
				}
				continue;
			};
			self.inputs.insert(slot, (resource, version));
			self.readers.entry((resource, version)).or_default().push(id);
			if input.access == Access::Write && input.kind.is_texture() {
				if let Some(other) = self.writers.insert((resource, version), id) {
					let name = self.resources[resource.0].name.clone();
					self.error(
						Some(id),
						format!("{name} is drawn into by {other} and by this node: one must come after the other"),
					);
				}
				after.insert(&input.name, (resource, version + 1));
			} else {
				after.insert(&input.name, (resource, version));
			}
		}
		for output in &pass.outputs {
			let slot = (id, output.name.to_string());
			match &output.source {
				OutputSource::Create(texture) => {
					let size = self.size(id, &output.name, &texture.size);
					let resource = self.add(
						ResourceKind::Texture {
							format: texture.format,
							size,
							physical: None,
						},
						&slot,
					);
					self.created.insert(resource, id);
					self.outputs.insert(slot, (resource, 0));
				}
				OutputSource::PassThrough(input) => {
					if let Some(&bound) = after.get(input.as_ref()) {
						self.outputs.insert(slot, bound);
					} else if pass.input_named(input).is_none() {
						self.error(
							Some(id),
							format!(
								"the output {:?} passes on {input:?}, which is not an input",
								output.name
							),
						);
					}
				}
				OutputSource::Data => {
					let resource = self.add(ResourceKind::Data, &slot);
					self.outputs.insert(slot, (resource, 0));
				}
			}
		}
	}

	fn add(&mut self, kind: ResourceKind, (node, slot): &Slot) -> ResourceId {
		self.resources.push(Resource {
			kind,
			name: format!("{node}.{slot}"),
		});
		ResourceId(self.resources.len() - 1)
	}

	/// The size of a texture the output `output` of `id` creates.
	fn size(&mut self, id: NodeId, output: &str, rule: &SizeRule) -> ResolvedSize {
		match rule {
			SizeRule::View { scale } => ResolvedSize::View(scale.to_bits()),
			SizeRule::Absolute(width, height) => ResolvedSize::Absolute(*width, *height),
			SizeRule::FromParam(name) => match self.params.get(&id).and_then(|p| p.get(name)) {
				Some(ParamValue::Int(side)) => {
					let side = u32::try_from(*side).unwrap_or(1);
					ResolvedSize::Absolute(side, side)
				}
				Some(ParamValue::Vec2([width, height])) => ResolvedSize::Absolute(pixels(*width), pixels(*height)),
				_ => {
					self.error(
						Some(id),
						format!("the size of {output:?} is the parameter {name:?}, which is not an Int or a Vec2"),
					);
					ResolvedSize::Absolute(1, 1)
				}
			},
		}
	}

	/// Step 5: the order. The links, and the readers of a version before the
	/// one that draws over it.
	fn order(&mut self, mut edges: BTreeSet<(NodeId, NodeId)>) -> Option<Vec<NodeId>> {
		for (version, writer) in &self.writers {
			for &reader in self.readers.get(version).into_iter().flatten() {
				if reader != *writer {
					edges.insert((reader, *writer));
				}
			}
		}
		self.sort(
			&edges,
			"a pass must read a texture before another draws into it, and also needs what that one made (a cycle):",
		)
	}

	/// An output node takes the target of the view, which is made once.
	fn check_outputs(&mut self) {
		let mut problems = Vec::new();
		for (&id, (_, pass)) in &self.nodes {
			if pass.category != PassCategory::Output {
				continue;
			}
			for input in pass.inputs.iter().filter(|input| input.kind.is_texture()) {
				if let Some(&(resource, _)) = self.inputs.get(&(id, input.name.to_string()))
					&& !self.resources[resource.0].is_view_target()
				{
					problems.push((
						Some(id),
						format!(
							"{:?} is given {}, which is not the target of the view",
							input.name, self.resources[resource.0].name
						),
					));
				}
			}
		}
		let targets: Vec<&str> = self
			.resources
			.iter()
			.filter(|r| r.is_view_target())
			.map(|r| r.name.as_str())
			.collect();
		if targets.len() > 1 {
			problems.push((
				None,
				format!(
					"the target of the view is created more than once: {}",
					targets.join(", ")
				),
			));
		}
		for (node, message) in problems {
			self.error(node, message);
		}
	}

	/// Step 6: a physical texture for every texture of the graph. Two share
	/// one when they are alike and one is done before the other begins.
	fn place_textures(&mut self, order: &[NodeId]) -> Vec<(FormatRule, ResolvedSize)> {
		let position: HashMap<NodeId, usize> = order.iter().enumerate().map(|(at, &id)| (id, at)).collect();
		let mut last_use: HashMap<ResourceId, usize> = HashMap::new();
		for ((id, _), (resource, _)) in self.inputs.iter().chain(self.outputs.iter()) {
			let at = position.get(id).copied().unwrap_or(0);
			let last = last_use.entry(*resource).or_insert(at);
			*last = (*last).max(at);
		}
		let mut by_start: Vec<(usize, ResourceId)> = self
			.created
			.iter()
			.map(|(&resource, id)| (position.get(id).copied().unwrap_or(0), resource))
			.collect();
		by_start.sort_unstable();
		// Each physical texture, with the last position it is used at.
		let mut physical: Vec<((FormatRule, ResolvedSize), usize)> = Vec::new();
		for (start, resource) in by_start {
			let ResourceKind::Texture {
				format,
				size,
				physical: slot,
			} = &mut self.resources[resource.0].kind
			else {
				continue;
			};
			if *format == FormatRule::ViewTarget {
				continue;
			}
			let key = (*format, *size);
			let end = last_use.get(&resource).copied().unwrap_or(start);
			let index = physical
				.iter()
				.position(|(k, busy)| *k == key && *busy < start)
				.unwrap_or_else(|| {
					physical.push((key, 0));
					physical.len() - 1
				});
			physical[index].1 = end;
			*slot = Some(index);
		}
		physical.into_iter().map(|(key, _)| key).collect()
	}

	/// The instances of the passes, in `order`, with their parameters set.
	fn instances(&mut self, order: Vec<NodeId>) -> Vec<CompiledNode> {
		let mut compiled = Vec::with_capacity(order.len());
		for id in order {
			let (Some(pass), Some(values)) = (
				self.nodes.get(&id).map(|(_, pass)| Arc::clone(pass)),
				self.params.remove(&id),
			) else {
				continue;
			};
			let Some(mut instance) = self.passes.make(&pass.type_name) else {
				continue;
			};
			if let Err(e) = instance.set_params(&values) {
				self.error(Some(id), format!("the parameters: {e}"));
				continue;
			}
			let bind = |slots: &HashMap<Slot, Version>, outputs: bool| -> HashMap<String, Binding> {
				slots
					.iter()
					.filter(|((node, _), _)| *node == id)
					.map(|((_, name), &(resource, _))| {
						let clear = outputs
							&& pass
								.output_named(name)
								.is_some_and(|o| matches!(o.source, OutputSource::Create(_)));
						(name.clone(), Binding { resource, clear })
					})
					.collect()
			};
			compiled.push(CompiledNode {
				id,
				label: format!("{} {id}", pass.display_name),
				pass: instance,
				params: values,
				inputs: bind(&self.inputs, false),
				outputs: bind(&self.outputs, true),
				state: NodeState::New,
				reported: false,
				bind_groups: BindGroupCache::default(),
				view_states: HashMap::new(),
			});
		}
		compiled
	}
}

/// The nodes in an order where every edge goes forward; ties go to the
/// smaller id. `Err`: the nodes on or after a cycle.
fn sort(nodes: impl Iterator<Item = NodeId>, edges: &BTreeSet<(NodeId, NodeId)>) -> Result<Vec<NodeId>, Vec<NodeId>> {
	let mut incoming: BTreeMap<NodeId, usize> = nodes.map(|id| (id, 0)).collect();
	for (_, to) in edges {
		if let Some(count) = incoming.get_mut(to) {
			*count += 1;
		}
	}
	let mut ready: BTreeSet<NodeId> = incoming.iter().filter(|(_, n)| **n == 0).map(|(&id, _)| id).collect();
	let mut order = Vec::with_capacity(incoming.len());
	while let Some(id) = ready.pop_first() {
		order.push(id);
		for (_, to) in edges.range((id, NodeId(0))..=(id, NodeId(u32::MAX))) {
			if let Some(count) = incoming.get_mut(to) {
				*count -= 1;
				if *count == 0 {
					ready.insert(*to);
				}
			}
		}
	}
	if order.len() == incoming.len() {
		Ok(order)
	} else {
		Err(incoming.into_keys().filter(|id| !order.contains(id)).collect())
	}
}

/// A cycle among `stuck` (the nodes [`sort`] could not order, which are on a
/// cycle or after one): its nodes in the order of the edges, so that the last
/// one has an edge to the first. Depth-first, with a node in one of three
/// states: not seen, on the path, done; an edge to a node on the path closes a
/// cycle.
fn find_cycle(stuck: &[NodeId], edges: &BTreeSet<(NodeId, NodeId)>) -> Vec<NodeId> {
	let successors = |id: NodeId| -> Vec<NodeId> {
		edges
			.range((id, NodeId(0))..=(id, NodeId(u32::MAX)))
			.map(|&(_, to)| to)
			.filter(|to| stuck.contains(to))
			.collect()
	};
	let mut done: BTreeSet<NodeId> = BTreeSet::new();
	for &start in stuck {
		if done.contains(&start) {
			continue;
		}
		// The path from `start`, each node with the successors still to visit.
		let mut path: Vec<(NodeId, Vec<NodeId>)> = vec![(start, successors(start))];
		while let Some((node, next)) = path.last_mut() {
			let node = *node;
			let Some(to) = next.pop() else {
				done.insert(node);
				path.pop();
				continue;
			};
			if let Some(at) = path.iter().position(|(id, _)| *id == to) {
				return path[at..].iter().map(|(id, _)| *id).collect();
			}
			if !done.contains(&to) {
				path.push((to, successors(to)));
			}
		}
	}
	// Not reached for nodes `sort` gave up on; the nodes themselves are the
	// best that can be said.
	stuck.to_vec()
}

/// `#1 → #2 → #1`.
fn describe_cycle(cycle: &[NodeId]) -> String {
	let mut path: Vec<String> = cycle.iter().map(ToString::to_string).collect();
	path.extend(cycle.first().map(ToString::to_string));
	path.join(" → ")
}
