//! `GraphRegistry` and `GraphId`: the graphs of the renderer, and the built-in
//! one that stands in for a broken or missing graph.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zenith_error::ZResult;

use super::{
	compile::{CompiledGraph, Diagnostic, Severity, compile},
	desc::GraphDesc,
	pass::PassRegistry,
};
use crate::assets::AssetId;

/// A graph: the id of its asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphId(pub AssetId);

impl GraphId {
	/// The graph built into the engine (`builtin_graph.json`).
	pub const BUILTIN: Self = Self(AssetId::from_bytes([
		0xa8, 0x99, 0x40, 0x19, 0x68, 0xfe, 0x4b, 0x16, 0xa2, 0x2e, 0x92, 0xae, 0x91, 0x4b, 0xca, 0x9f,
	]));

	/// The graph the viewport of the editor runs: the built-in one with the
	/// grid, the gizmos and the outline (`editor_graph.json`).
	pub const EDITOR: Self = Self(AssetId::from_bytes([
		0xe7, 0xd1, 0xc3, 0xa2, 0x5b, 0x94, 0x4f, 0x0e, 0x8a, 0x16, 0x3c, 0x2d, 0x9b, 0x7f, 0x4e, 0x51,
	]));
}

/// The text of the built-in graph.
pub const BUILTIN_GRAPH: &str = include_str!("builtin_graph.json");

/// The text of the graph of the editor: the built-in one with its own passes
/// (`editor_graph.json`).
pub const EDITOR_GRAPH: &str = include_str!("editor_graph.json");

/// A graph of the registry: what it is, and what runs.
pub struct GraphEntry {
	pub desc: GraphDesc,
	/// The last version that compiled: it goes on running while an edit does
	/// not compile. `None`: no version ever compiled.
	pub compiled: Option<CompiledGraph>,
	/// What the last compiling found.
	pub diagnostics: Vec<Diagnostic>,
}

/// The graphs of the renderer, compiled.
#[derive(Default)]
pub struct GraphRegistry {
	graphs: HashMap<GraphId, GraphEntry>,
	main: Option<GraphId>,
}

impl GraphRegistry {
	pub fn new() -> Self {
		Self::default()
	}

	/// Puts `desc` in (in place of the graph with its id) and compiles it. If
	/// it does not compile, the version that did before goes on running.
	/// Returns what compiling found.
	pub fn set(&mut self, desc: GraphDesc, passes: &PassRegistry) -> &[Diagnostic] {
		let id = GraphId(desc.uuid);
		let (compiled, diagnostics) = match compile(&desc, passes) {
			Ok(compiled) => {
				let diagnostics = compiled.diagnostics().to_vec();
				(Some(compiled), diagnostics)
			}
			Err(diagnostics) => (None, diagnostics),
		};
		for diagnostic in &diagnostics {
			match diagnostic.severity {
				Severity::Warning => zenith_log::warn!("The render graph {:?}: {diagnostic}", desc.name),
				Severity::Error => zenith_log::error!("The render graph {:?}: {diagnostic}", desc.name),
			}
		}
		let entry = self.graphs.entry(id).or_insert_with(|| GraphEntry {
			desc: desc.clone(),
			compiled: None,
			diagnostics: Vec::new(),
		});
		entry.desc = desc;
		if compiled.is_some() {
			entry.compiled = compiled;
		}
		entry.diagnostics = diagnostics;
		&entry.diagnostics
	}

	/// Puts the built-in graph in.
	///
	/// # Errors
	/// The text of the built-in graph cannot be read (a bug of the engine).
	pub fn set_builtin(&mut self, passes: &PassRegistry) -> ZResult<&[Diagnostic]> {
		Ok(self.set(GraphDesc::parse(BUILTIN_GRAPH)?, passes))
	}

	/// Puts the graph of the editor in.
	///
	/// # Errors
	/// The text of the graph cannot be read (a bug of the engine).
	pub fn set_editor(&mut self, passes: &PassRegistry) -> ZResult<&[Diagnostic]> {
		Ok(self.set(GraphDesc::parse(EDITOR_GRAPH)?, passes))
	}

	/// Compiles every graph again (the passes changed: a plugin, a script).
	pub fn recompile(&mut self, passes: &PassRegistry) {
		let descs: Vec<GraphDesc> = self.graphs.values().map(|entry| entry.desc.clone()).collect();
		for desc in descs {
			self.set(desc, passes);
		}
	}

	pub fn remove(&mut self, id: GraphId) -> Option<GraphEntry> {
		if self.main == Some(id) {
			self.main = None;
		}
		self.graphs.remove(&id)
	}

	pub fn get(&self, id: GraphId) -> Option<&GraphEntry> {
		self.graphs.get(&id)
	}

	/// The graph of the cameras that do not name one. `None`: the built-in
	/// one.
	pub const fn main(&self) -> Option<GraphId> {
		self.main
	}

	pub const fn set_main(&mut self, main: Option<GraphId>) {
		self.main = main;
	}

	/// The graph a camera that asks for `wanted` runs: that one if it runs,
	/// else the main graph, else the built-in one.
	pub fn resolve(&mut self, wanted: Option<GraphId>) -> Option<(GraphId, &mut CompiledGraph)> {
		let id = [wanted, self.main, Some(GraphId::BUILTIN)]
			.into_iter()
			.flatten()
			.find(|id| self.graphs.get(id).is_some_and(|entry| entry.compiled.is_some()))?;
		let compiled = self.graphs.get_mut(&id)?.compiled.as_mut()?;
		Some((id, compiled))
	}

	pub fn len(&self) -> usize {
		self.graphs.len()
	}

	pub fn is_empty(&self) -> bool {
		self.graphs.is_empty()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_built_in_graph_has_its_own_id() -> ZResult<()> {
		assert_eq!(GraphId(GraphDesc::parse(BUILTIN_GRAPH)?.uuid), GraphId::BUILTIN);
		Ok(())
	}

	#[test]
	fn the_editor_graph_has_its_own_id_and_compiles() -> ZResult<()> {
		assert_eq!(GraphId(GraphDesc::parse(EDITOR_GRAPH)?.uuid), GraphId::EDITOR);
		let mut passes = PassRegistry::new();
		crate::passes::register(&mut passes);
		let mut graphs = GraphRegistry::new();
		assert_eq!(graphs.set_editor(&passes)?, []);
		assert!(
			graphs
				.get(GraphId::EDITOR)
				.is_some_and(|entry| entry.compiled.is_some())
		);
		Ok(())
	}
}
