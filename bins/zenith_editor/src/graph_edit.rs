//! The data side of the graph editor: the numbers imnodes knows nodes and pins
//! by, what may be linked to what, where nodes go, and how parameters are read
//! and written. No UI here, so it can be tested.

// The numbers are ids of nodes and pins: small, and made to fit an `i32`.
#![allow(clippy::cast_possible_wrap, clippy::cast_sign_loss, clippy::cast_possible_truncation)]

use std::collections::HashMap;

use zenith_error::ZResult;
use zenith_renderer::{
	assets::AssetId,
	graph::{
		GraphDesc, NodeId, PassCategory, PassDesc, PassRegistry, SlotKind, SlotRef,
		desc::{NodeDesc, NodeEditor},
		pass::{ParamDesc, ParamValue},
	},
};

/// How far apart the columns and the rows of an automatic layout are.
const COLUMN: f32 = 340.0;
const ROW: f32 = 190.0;

/// The pins of a node are numbered from its id: its number in the high bits,
/// the slot in the middle and the side in the lowest.
pub const fn pin_id(node: NodeId, slot: usize, output: bool) -> i32 {
	((node.0 as i32) << 8) | ((slot as i32) << 1) | (output as i32)
}

/// The inverse of [`pin_id`]: the node, the slot and whether it is an output.
pub const fn split_pin(pin: i32) -> (NodeId, usize, bool) {
	(NodeId((pin >> 8) as u32), ((pin >> 1) & 0x7f) as usize, pin & 1 == 1)
}

/// The two ends of a link the user drew, from an output to an input, or why it
/// is not allowed. The ends may come in either order.
///
/// # Errors
/// The reason, in words for the user.
pub fn link_ends(desc: &GraphDesc, passes: &PassRegistry, a: i32, b: i32) -> Result<(SlotRef, SlotRef), String> {
	let ((from_node, from_slot, from_out), (to_node, to_slot, to_out)) = (split_pin(a), split_pin(b));
	let ((out_node, out_slot), (in_node, in_slot)) = match (from_out, to_out) {
		(true, false) => ((from_node, from_slot), (to_node, to_slot)),
		(false, true) => ((to_node, to_slot), (from_node, from_slot)),
		_ => return Err("A link goes from an output to an input.".to_string()),
	};
	if out_node == in_node {
		return Err("A node cannot be linked to itself.".to_string());
	}
	let pass_of = |node: NodeId| -> Result<std::sync::Arc<PassDesc>, String> {
		let node = desc.node(node).ok_or_else(|| format!("There is no node {node}."))?;
		passes
			.get(&node.pass)
			.cloned()
			.ok_or_else(|| format!("The pass {} is not known.", node.pass))
	};
	let (out_pass, in_pass) = (pass_of(out_node)?, pass_of(in_node)?);
	let output = out_pass.outputs.get(out_slot).ok_or("That output does not exist.")?;
	let input = in_pass.inputs.get(in_slot).ok_or("That input does not exist.")?;
	if output.kind != input.kind {
		return Err(format!(
			"{} cannot go into {}: they carry different things.",
			kind_name(&output.kind),
			kind_name(&input.kind)
		));
	}
	Ok((
		SlotRef::new(out_node, output.name.to_string()),
		SlotRef::new(in_node, input.name.to_string()),
	))
}

pub fn kind_name(kind: &SlotKind) -> String {
	match kind {
		SlotKind::Color => "Color".to_string(),
		SlotKind::Depth => "Depth".to_string(),
		SlotKind::Data(name) => format!("Data ({name})"),
	}
}

/// The color of a pin by what it carries.
pub const fn kind_color(kind: &SlotKind) -> [f32; 4] {
	match kind {
		SlotKind::Color => [0.35, 0.65, 0.95, 1.0],
		SlotKind::Depth => [0.75, 0.75, 0.75, 1.0],
		SlotKind::Data(_) => [0.95, 0.75, 0.3, 1.0],
	}
}

/// The color of the title of a node by the kind of its pass.
pub const fn category_color(category: PassCategory) -> [f32; 4] {
	match category {
		PassCategory::Input => [0.25, 0.5, 0.3, 1.0],
		PassCategory::Geometry => [0.3, 0.4, 0.7, 1.0],
		PassCategory::Lighting => [0.75, 0.6, 0.25, 1.0],
		PassCategory::Post => [0.25, 0.6, 0.65, 1.0],
		PassCategory::Output => [0.65, 0.3, 0.3, 1.0],
		PassCategory::Debug => [0.5, 0.35, 0.65, 1.0],
	}
}

/// Gives a position to every node that has none: columns by the longest way
/// from a node with no inputs, rows in the order of the nodes.
pub fn auto_layout(desc: &mut GraphDesc) {
	let mut depth: HashMap<NodeId, u32> = desc.nodes.iter().map(|node| (node.id, 0)).collect();
	// The graph has no cycles when it compiles, but one that does not must
	// not hang the editor: the passes are bounded.
	for _ in 0..desc.nodes.len() {
		let mut moved = false;
		for link in &desc.links {
			let from = depth.get(&link.from.0).copied().unwrap_or(0);
			if let Some(to) = depth.get_mut(&link.to.0)
				&& *to < from + 1
			{
				*to = from + 1;
				moved = true;
			}
		}
		if !moved {
			break;
		}
	}
	let mut rows: HashMap<u32, u32> = HashMap::new();
	for node in &mut desc.nodes {
		let column = depth.get(&node.id).copied().unwrap_or(0);
		let row = rows.entry(column).or_insert(0);
		if node.editor.is_none() {
			node.editor = Some(NodeEditor {
				pos: [column as f32 * COLUMN, *row as f32 * ROW],
				extra: serde_json::Map::new(),
			});
		}
		*row += 1;
	}
}

/// The positions of the nodes in the state imnodes saves as text
/// (`[node.5]` then `origin=x,y`), by node.
pub fn parse_positions(ini: &str) -> HashMap<i32, [f32; 2]> {
	let mut found = HashMap::new();
	let mut current: Option<i32> = None;
	for line in ini.lines() {
		let line = line.trim();
		if let Some(section) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
			current = section.strip_prefix("node.").and_then(|id| id.parse().ok());
		} else if let (Some(node), Some(origin)) = (current, line.strip_prefix("origin=")) {
			let mut parts = origin.split(',').map(|part| part.trim().parse::<f32>());
			if let (Some(Ok(x)), Some(Ok(y))) = (parts.next(), parts.next()) {
				found.insert(node, [x, y]);
			}
		}
	}
	found
}

/// Writes the positions the editor shows back into the graph. `true` if any
/// node moved.
pub fn store_positions(desc: &mut GraphDesc, positions: &HashMap<i32, [f32; 2]>) -> bool {
	let mut moved = false;
	for node in &mut desc.nodes {
		let Some(&pos) = positions.get(&(node.id.0 as i32)) else {
			continue;
		};
		#[allow(clippy::float_cmp)] // a position that did not move is the same number
		match &mut node.editor {
			Some(editor) if editor.pos == pos => {}
			Some(editor) => {
				editor.pos = pos;
				moved = true;
			}
			None => {
				node.editor = Some(NodeEditor {
					pos,
					extra: serde_json::Map::new(),
				});
				moved = true;
			}
		}
	}
	moved
}

/// The value of a parameter of a node: what the file says, else the default.
pub fn param_value(node: &NodeDesc, param: &ParamDesc) -> ParamValue {
	node.params
		.get(param.name.as_ref())
		.and_then(|json| ParamValue::from_json(&param.ty, json))
		.unwrap_or_else(|| param.default.clone())
}

/// Puts a parameter into a node. A value that is the default is taken out of
/// the file, so the files stay short.
pub fn set_param(node: &mut NodeDesc, param: &ParamDesc, value: &ParamValue) {
	if *value == param.default {
		node.params.remove(param.name.as_ref());
	} else {
		node.params.insert(param.name.to_string(), value.to_json());
	}
}

/// A copy of a graph with an id of its own and a name.
///
/// # Errors
/// The system could not make an id.
pub fn copy_graph(desc: &GraphDesc, name: &str) -> ZResult<GraphDesc> {
	let mut copy = desc.clone();
	copy.uuid = AssetId::generate()?;
	copy.name = name.to_string();
	Ok(copy)
}

/// A file name for a graph: letters, digits, `-` and `_`.
pub fn file_stem(name: &str) -> String {
	let stem: String = name
		.trim()
		.chars()
		.map(|c| {
			if c.is_alphanumeric() || c == '-' || c == '_' {
				c
			} else {
				'_'
			}
		})
		.collect();
	if stem.is_empty() { "Graph".to_string() } else { stem }
}

#[cfg(test)]
mod tests {
	use zenith_renderer::graph::pass::{ParamDesc, ParamValue};

	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	fn builtin() -> Result<(GraphDesc, PassRegistry), Box<dyn std::error::Error>> {
		let mut passes = PassRegistry::new();
		zenith_renderer::passes::register(&mut passes);
		Ok((
			GraphDesc::parse(zenith_renderer::graph::registry::BUILTIN_GRAPH)?,
			passes,
		))
	}

	#[test]
	fn pins_are_told_apart_and_come_back() {
		let pin = pin_id(NodeId(13), 3, true);
		assert_eq!(split_pin(pin), (NodeId(13), 3, true));
		assert_ne!(pin, pin_id(NodeId(13), 3, false));
		assert_ne!(pin, pin_id(NodeId(14), 3, true));
	}

	#[test]
	fn a_link_is_checked_in_either_direction() -> TestResult {
		let (desc, passes) = builtin()?;
		// 1 is the camera input (output 0: camera), 3 is opaque (input 0:
		// camera).
		let out = pin_id(NodeId(1), 0, true);
		let input = pin_id(NodeId(3), 0, false);
		let (from, to) = link_ends(&desc, &passes, out, input)?;
		assert_eq!((from.0, from.1.as_str()), (NodeId(1), "camera"));
		assert_eq!((to.0, to.1.as_str()), (NodeId(3), "camera"));
		let flipped = link_ends(&desc, &passes, input, out)?;
		assert_eq!(flipped, (from, to), "the end the user started from does not matter");
		Ok(())
	}

	#[test]
	fn a_link_must_join_what_fits() -> TestResult {
		let (desc, passes) = builtin()?;
		let camera_out = pin_id(NodeId(1), 0, true);
		let camera_in = pin_id(NodeId(3), 0, false);
		// An output to an output, an input to an input, a node to itself.
		assert!(link_ends(&desc, &passes, camera_out, pin_id(NodeId(2), 0, true)).is_err());
		assert!(link_ends(&desc, &passes, camera_in, pin_id(NodeId(8), 0, false)).is_err());
		assert!(link_ends(&desc, &passes, pin_id(NodeId(3), 0, true), camera_in).is_err());
		// Camera data into the meshes input: different things.
		let meshes_in = pin_id(NodeId(3), 1, false);
		let error = link_ends(&desc, &passes, camera_out, meshes_in)
			.err()
			.ok_or("it went")?;
		assert!(error.contains("different"), "{error}");
		// A node that is not there.
		assert!(link_ends(&desc, &passes, pin_id(NodeId(99), 0, true), camera_in).is_err());
		Ok(())
	}

	#[test]
	fn nodes_without_a_place_get_columns_by_depth() -> TestResult {
		let (mut desc, _) = builtin()?;
		for node in &mut desc.nodes {
			node.editor = None;
		}
		auto_layout(&mut desc);
		let x = |id: u32| desc.node(NodeId(id)).and_then(|n| n.editor.as_ref()).map(|e| e.pos[0]);
		assert_eq!(x(1), Some(0.0), "an input is in the first column");
		assert!(x(3) > x(1), "opaque is right of the camera");
		assert!(x(5) > x(4), "the output is right of the tone mapping");
		// What has a place keeps it.
		let before = desc.clone();
		auto_layout(&mut desc);
		assert_eq!(desc, before);
		Ok(())
	}

	#[test]
	fn positions_are_read_from_the_state_of_imnodes_and_stored() -> TestResult {
		let ini = "[node.1]\norigin=10.5,20\n\n[node.3]\norigin=-40.0,8.25\n[editor]\npanning=1,2\n";
		let positions = parse_positions(ini);
		assert_eq!(positions.get(&1), Some(&[10.5, 20.0]));
		assert_eq!(positions.get(&3), Some(&[-40.0, 8.25]));
		assert_eq!(positions.len(), 2);

		let (mut desc, _) = builtin()?;
		assert!(store_positions(&mut desc, &positions));
		assert!(!store_positions(&mut desc, &positions), "the same again moves nothing");
		assert_eq!(
			desc.node(NodeId(3)).and_then(|n| n.editor.as_ref()).map(|e| e.pos),
			Some([-40.0, 8.25])
		);
		Ok(())
	}

	#[test]
	fn a_parameter_that_is_the_default_is_not_written() -> TestResult {
		let (mut desc, passes) = builtin()?;
		let pass = passes.get("zenith.tonemap").ok_or("no tonemap")?;
		let exposure: &ParamDesc = pass.params.iter().find(|p| p.name == "exposure").ok_or("no exposure")?;
		let node = desc.node_mut(NodeId(4)).ok_or("no node")?;
		assert_eq!(param_value(node, exposure), ParamValue::Float(1.0));
		set_param(node, exposure, &ParamValue::Float(2.5));
		assert_eq!(param_value(node, exposure), ParamValue::Float(2.5));
		set_param(node, exposure, &ParamValue::Float(1.0));
		assert!(!node.params.contains_key("exposure"), "back to the default: gone");
		Ok(())
	}

	#[test]
	fn a_copy_has_its_own_id_and_names_become_file_names() -> TestResult {
		let (desc, _) = builtin()?;
		let copy = copy_graph(&desc, "My graph")?;
		assert_ne!(copy.uuid, desc.uuid);
		assert_eq!(copy.name, "My graph");
		assert_eq!(copy.nodes, desc.nodes);
		assert_eq!(file_stem(" My graph/2 "), "My_graph_2");
		assert_eq!(file_stem("  "), "Graph");
		Ok(())
	}
}
