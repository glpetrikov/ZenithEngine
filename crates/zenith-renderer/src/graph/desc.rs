//! `GraphDesc`: the nodes, links and parameters of a graph, as JSON holds them
//! and the editor edits them.
//!
//! It is plain data: a node of a pass the registry does not know (a plugin
//! that is not loaded) stays in it as it was, and is written back on save.
//! What is not a field here (in a node, in its `editor` part, at the top) is
//! kept too.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::Project;

use crate::assets::{AssetId, AssetKind, AssetRegistry, json};

/// A node of a graph. Stable: never reused in the same graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

impl std::fmt::Display for NodeId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "#{}", self.0)
	}
}

/// Where a node is in the graph editor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeEditor {
	pub pos: [f32; 2],
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeDesc {
	pub id: NodeId,
	/// The `type_name` of its pass.
	pub pass: String,
	#[serde(default, skip_serializing_if = "Map::is_empty")]
	pub params: Map<String, Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub editor: Option<NodeEditor>,
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}

/// A slot of a node: `[id, "name"]` in the file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SlotRef(pub NodeId, pub String);

impl SlotRef {
	pub fn new(node: NodeId, slot: impl Into<String>) -> Self {
		Self(node, slot.into())
	}
}

impl std::fmt::Display for SlotRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}.{}", self.0, self.1)
	}
}

/// From an output to an input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkDesc {
	pub from: SlotRef,
	pub to: SlotRef,
}

/// A render graph, as its file (`.zenithgraph`) holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphDesc {
	pub uuid: AssetId,
	#[serde(rename = "type")]
	pub kind: String,
	pub version: u32,
	#[serde(default)]
	pub name: String,
	#[serde(default)]
	pub nodes: Vec<NodeDesc>,
	#[serde(default)]
	pub links: Vec<LinkDesc>,
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}

impl GraphDesc {
	pub const VERSION: u32 = 1;

	/// An empty graph.
	pub fn new(uuid: AssetId, name: impl Into<String>) -> Self {
		Self {
			uuid,
			kind: AssetKind::RenderGraph.type_name().into(),
			version: Self::VERSION,
			name: name.into(),
			nodes: Vec::new(),
			links: Vec::new(),
			extra: Map::new(),
		}
	}

	/// # Errors
	/// `Json` for text that is not a graph, `InvalidAsset` for one of another
	/// type or a newer version.
	pub fn parse(text: &str) -> ZResult<Self> {
		json::read_id(text, AssetKind::RenderGraph)?;
		let graph: Self = serde_json::from_str(text).map_err(|e| ZenithError::Json(e.to_string()))?;
		if graph.version > Self::VERSION {
			return Err(ZenithError::InvalidAsset(format!(
				"the graph is of version {}, this engine reads up to {}",
				graph.version,
				Self::VERSION
			)));
		}
		Ok(graph)
	}

	/// # Errors
	/// `Json` if it cannot be written (it always can).
	pub fn to_text(&self) -> ZResult<String> {
		json::to_text(self)
	}

	/// The graph `id` of a project.
	///
	/// # Errors
	/// `AssetNotFound` for an id the registry does not have, `InvalidAsset`
	/// for an asset that is not a graph, as [`GraphDesc::parse`] for one that
	/// cannot be read, or an error of the project.
	pub fn load(project: &Project, registry: &AssetRegistry, id: AssetId) -> ZResult<Self> {
		let entry = registry
			.get(id)
			.ok_or_else(|| ZenithError::AssetNotFound(format!("no asset has the id {id}")))?;
		if entry.kind != AssetKind::RenderGraph {
			return Err(ZenithError::InvalidAsset(format!(
				"{} is a {:?}, not a render graph",
				entry.path, entry.kind
			)));
		}
		Self::parse(&project.read_string(&entry.path)?).map_err(|e| match e {
			ZenithError::Json(message) => ZenithError::Json(format!("{}: {message}", entry.path)),
			ZenithError::InvalidAsset(message) => ZenithError::InvalidAsset(format!("{}: {message}", entry.path)),
			other => other,
		})
	}

	pub fn node(&self, id: NodeId) -> Option<&NodeDesc> {
		self.nodes.iter().find(|node| node.id == id)
	}

	pub fn node_mut(&mut self, id: NodeId) -> Option<&mut NodeDesc> {
		self.nodes.iter_mut().find(|node| node.id == id)
	}

	/// An id no node has had: one more than the biggest.
	pub fn next_id(&self) -> NodeId {
		NodeId(self.nodes.iter().map(|node| node.id.0 + 1).max().unwrap_or(1))
	}

	/// Adds a node of the pass `type_name`, with its default parameters.
	pub fn add_node(&mut self, type_name: impl Into<String>, pos: [f32; 2]) -> NodeId {
		let id = self.next_id();
		self.nodes.push(NodeDesc {
			id,
			pass: type_name.into(),
			params: Map::new(),
			editor: Some(NodeEditor { pos, extra: Map::new() }),
			extra: Map::new(),
		});
		id
	}

	/// Removes a node and its links.
	pub fn remove_node(&mut self, id: NodeId) {
		self.nodes.retain(|node| node.id != id);
		self.links.retain(|link| link.from.0 != id && link.to.0 != id);
	}

	/// Links `from` to `to`. An input has one link: the one it had goes.
	pub fn connect(&mut self, from: SlotRef, to: SlotRef) {
		self.disconnect(&to);
		self.links.push(LinkDesc { from, to });
	}

	/// Removes the link into the input `to`.
	pub fn disconnect(&mut self, to: &SlotRef) {
		self.links.retain(|link| &link.to != to);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const BUILTIN: &str = include_str!("builtin_graph.json");

	#[test]
	fn the_built_in_graph_is_read_and_written_back_as_it_was() -> ZResult<()> {
		let graph = GraphDesc::parse(BUILTIN)?;
		assert_eq!(graph.nodes.len(), 13);
		assert_eq!(graph.links.len(), 25);
		assert_eq!(
			graph.links.first().map(|link| link.from.clone()),
			Some(SlotRef::new(NodeId(1), "camera"))
		);
		assert_eq!(GraphDesc::parse(&graph.to_text()?)?, graph);
		Ok(())
	}

	#[test]
	fn what_the_engine_does_not_know_is_kept() -> ZResult<()> {
		let text = r#"{
			"uuid": "a8994019-68fe-4b16-a22e-92ae914bca9f", "type": "render_graph", "version": 1,
			"comment": "by hand",
			"nodes": [ { "id": 7, "pass": "plugin.fog", "params": { "density": 0.3 },
			             "editor": { "pos": [1, 2], "collapsed": true }, "color": "red" } ]
		}"#;
		let graph = GraphDesc::parse(text)?;
		let written = graph.to_text()?;
		for kept in ["\"comment\"", "\"collapsed\"", "\"color\"", "\"density\""] {
			assert!(written.contains(kept), "{kept} in {written}");
		}
		Ok(())
	}

	#[test]
	fn editing_keeps_one_link_per_input_and_ids_unused() -> ZResult<()> {
		let mut graph = GraphDesc::parse(BUILTIN)?;
		let fog = graph.add_node("plugin.fog", [0.0, 0.0]);
		assert_eq!(fog, NodeId(14));
		graph.connect(SlotRef::new(fog, "color"), SlotRef::new(NodeId(5), "color"));
		assert_eq!(
			graph
				.links
				.iter()
				.filter(|link| link.to == SlotRef::new(NodeId(5), "color"))
				.count(),
			1,
			"the old link into the output went"
		);
		graph.remove_node(fog);
		assert!(graph.links.iter().all(|link| link.from.0 != fog));
		assert_eq!(graph.links.len(), 24);
		Ok(())
	}

	#[test]
	fn another_type_or_a_newer_version_is_refused() {
		let material = r#"{ "uuid": "a8994019-68fe-4b16-a22e-92ae914bca9f", "type": "material" }"#;
		assert!(GraphDesc::parse(material).is_err());
		let newer = r#"{ "uuid": "a8994019-68fe-4b16-a22e-92ae914bca9f", "type": "render_graph", "version": 9 }"#;
		assert!(GraphDesc::parse(newer).is_err());
	}
}
