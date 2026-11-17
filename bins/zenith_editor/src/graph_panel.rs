//! The graph editor: a panel that shows a render graph as nodes and links,
//! edits it, compiles it after every edit and saves it.
//!
//! The renderer lives in the world that is shown (see `scene_view`), so the
//! panel works on the graphs of that renderer. What is edited is a copy
//! (`working`) of the description; every change puts the copy back with
//! `Renderer::set_graph`, which compiles it and keeps the last version that
//! worked while the new one has errors.

// The numbers are ids of nodes and pins, and sizes of a panel: small.
#![allow(clippy::cast_possible_wrap, clippy::cast_sign_loss, clippy::cast_possible_truncation)]

use std::collections::HashMap;

use dear_imgui_rs::{Condition, Key, MouseButton, Ui};
use dear_imnodes::{self as imnodes, ColorElement, ImNodesExt, LinkId, NodeId as ImNode, PinId, PinShape};
use zenith_filesystem::{Project, ZPath};
use zenith_renderer::{
	Renderer,
	assets::{AssetKind, AssetRegistry},
	graph::{
		Diagnostic, GraphDesc, GraphId, NodeId, ParamDesc, ParamType, ParamValue, PassCategory, PassDesc, PassRegistry,
		Severity, SlotRef,
	},
};

use crate::{
	editor::Editor,
	graph_edit::{
		auto_layout, category_color, copy_graph, file_stem, kind_color, link_ends, param_value, parse_positions,
		pin_id, set_param, store_positions,
	},
};

const WINDOW: &str = "Render Graph";
const ADD_MENU: &str = "Add node";
const SAVE_AS: &str = "Save graph as";
/// The folder of the project the graphs are saved into.
const GRAPHS_DIR: &str = "Assets://Graphs";
/// Slots of the static attributes (the parameters) start here, past the pins.
const PARAM_SLOT: usize = 64;
const PARAM_WIDTH: f32 = 150.0;
const ERROR_COLOR: [f32; 4] = [0.85, 0.2, 0.2, 1.0];
const WARNING_COLOR: [f32; 4] = [0.9, 0.7, 0.2, 1.0];

/// The state of the graph editor.
pub struct GraphEditor {
	pub open: bool,
	/// The contexts of imnodes; made once the UI exists, let go of before it
	/// is gone.
	imnodes: Option<(imnodes::Context, imnodes::EditorContext)>,
	selected: GraphId,
	/// The graph as it is being edited.
	working: Option<GraphDesc>,
	/// The nodes of the graph get the positions the description holds.
	place_nodes: bool,
	/// Nodes just added: where to put them (a place on the screen).
	pending: Vec<(NodeId, [f32; 2])>,
	menu_at: [f32; 2],
	/// There are changes that are not saved.
	dirty: bool,
	save_as_name: String,
	notice: Option<String>,
}

impl Default for GraphEditor {
	fn default() -> Self {
		Self {
			open: true,
			imnodes: None,
			selected: GraphId::BUILTIN,
			working: None,
			place_nodes: false,
			pending: Vec::new(),
			menu_at: [0.0; 2],
			dirty: false,
			save_as_name: String::new(),
			notice: None,
		}
	}
}

impl GraphEditor {
	/// Makes the contexts of imnodes once the UI has its own.
	pub fn init(&mut self, imgui: &dear_imgui_rs::Context) {
		if self.imnodes.is_none() {
			let context = imnodes::Context::create(imgui);
			let editor = context.create_editor_context();
			self.imnodes = Some((context, editor));
		}
	}

	/// Lets go of imnodes: it must go before the UI does.
	pub fn shutdown(&mut self) {
		self.imnodes = None;
	}

	/// Forget the graph that was being edited (the world changed).
	pub fn reset(&mut self) {
		self.working = None;
		self.dirty = false;
		self.pending.clear();
	}
}

/// What the user did in a frame, applied once the nodes are drawn.
enum Edit {
	Param {
		node: NodeId,
		param: ParamDesc,
		value: ParamValue,
	},
	Link(SlotRef, SlotRef),
	Unlink(SlotRef),
	Remove(Vec<NodeId>),
	Add {
		pass: String,
		at: [f32; 2],
	},
}

/// What the panel reads of the renderer in a frame.
struct Snapshot {
	passes: PassRegistry,
	/// The graphs the renderer has: the id, the name and the file of a graph
	/// of the project.
	loaded: Vec<(GraphId, String, Option<ZPath>)>,
	/// The graphs of the project that the renderer does not have yet.
	unloaded: Vec<(GraphId, ZPath)>,
	diagnostics: Vec<Diagnostic>,
	main: Option<GraphId>,
}

fn snapshot(renderer: &Renderer, registry: &AssetRegistry, selected: GraphId) -> Snapshot {
	let graphs = renderer.graphs();
	let loaded: Vec<(GraphId, String, Option<ZPath>)> = graphs
		.ids()
		.into_iter()
		.filter_map(|id| {
			let entry = graphs.get(id)?;
			Some((id, entry.desc.name.clone(), registry.path(id.0).cloned()))
		})
		.collect();
	let unloaded = registry
		.iter()
		.filter(|(_, entry)| entry.kind == AssetKind::RenderGraph)
		.filter(|(id, _)| !loaded.iter().any(|(loaded_id, ..)| loaded_id.0 == *id))
		.map(|(id, entry)| (GraphId(id), entry.path.clone()))
		.collect();
	Snapshot {
		passes: renderer.passes().clone(),
		loaded,
		unloaded,
		diagnostics: graphs
			.get(selected)
			.map(|entry| entry.diagnostics.clone())
			.unwrap_or_default(),
		main: graphs.main(),
	}
}

/// The panel.
pub fn draw(editor: &mut Editor, ui: &Ui) {
	if !editor.graph.open {
		return;
	}
	let mut open = true;
	ui.window(WINDOW)
		.size([900.0, 520.0], Condition::FirstUseEver)
		.position([60.0, 120.0], Condition::FirstUseEver)
		.opened(&mut open)
		.build(|| body(editor, ui));
	editor.graph.open = open;
}

fn body(editor: &mut Editor, ui: &Ui) {
	let Editor {
		project,
		active_world,
		scene,
		graph,
		..
	} = editor;
	let files = project.as_ref().map(|project| std::sync::Arc::clone(&project.project));
	let (Some(project), Some(name)) = (project.as_mut(), active_world.as_ref()) else {
		ui.text_disabled("Open a project with a world to edit its render graphs");
		return;
	};
	let Some(entry) = project.worlds.get_mut(name) else {
		return;
	};
	let ecs = entry.world.world_mut();
	if !ecs.contains_resource::<Renderer>() {
		ui.text_disabled("The renderer is not ready yet");
		return;
	}
	let shot = {
		let renderer = ecs.resource::<Renderer>();
		let registry = ecs.resource::<AssetRegistry>();
		snapshot(renderer, registry, graph.selected)
	};
	if graph.working.is_none() {
		let desc = ecs
			.resource::<Renderer>()
			.graphs()
			.get(graph.selected)
			.map(|entry| entry.desc.clone());
		if let Some(mut desc) = desc {
			auto_layout(&mut desc);
			graph.working = Some(desc);
			graph.place_nodes = true;
			graph.dirty = false;
		}
	}

	let Some(files) = &files else { return };
	toolbar(ui, graph, scene, &shot, files, ecs);
	let Some(mut desc) = graph.working.take() else {
		ui.text_disabled("This graph is not there");
		return;
	};
	let mut edits: Vec<Edit> = Vec::new();
	let mut positions = HashMap::new();
	let height = (ui.content_region_avail()[1] - diagnostics_height(&shot.diagnostics)).max(120.0);
	ui.child_window("##nodes").size([0.0, height]).build(ui, || {
		nodes(ui, graph, &desc, &shot, &mut edits, &mut positions);
	});
	diagnostics(ui, &shot.diagnostics);
	if let Some(notice) = &graph.notice {
		ui.text_disabled(notice);
	}

	if store_positions(&mut desc, &positions) {
		graph.dirty = true;
	}
	let changed = apply(&mut desc, &shot.passes, edits, graph);
	if changed {
		graph.dirty = true;
		ecs.resource_mut::<Renderer>().set_graph(desc.clone());
	}
	graph.working = Some(desc);
}

fn diagnostics_height(diagnostics: &[Diagnostic]) -> f32 {
	(diagnostics.len().min(4) as f32).mul_add(20.0, 24.0)
}

fn diagnostics(ui: &Ui, diagnostics: &[Diagnostic]) {
	ui.separator();
	if diagnostics.is_empty() {
		ui.text_colored([0.4, 0.8, 0.4, 1.0], "The graph compiles");
		return;
	}
	for diagnostic in diagnostics {
		let color = match diagnostic.severity {
			Severity::Error => ERROR_COLOR,
			Severity::Warning => WARNING_COLOR,
		};
		ui.text_colored(color, diagnostic.to_string());
	}
}

/// The row above the nodes: which graph, and what can be done with it.
fn toolbar(
	ui: &Ui,
	graph: &mut GraphEditor,
	scene: &mut crate::scene_view::SceneView,
	shot: &Snapshot,
	project: &Project,
	ecs: &mut bevy_ecs::world::World,
) {
	let mut labels: Vec<String> = shot
		.loaded
		.iter()
		.map(|(id, name, file)| {
			let kind = match (file, GraphId::BUILTIN == *id || GraphId::EDITOR == *id) {
				(_, true) => "built in",
				(Some(_), _) => "project",
				(None, _) => "not saved",
			};
			format!("{name} ({kind})")
		})
		.collect();
	let first_unloaded = labels.len();
	labels.extend(shot.unloaded.iter().map(|(_, path)| format!("{path} (not loaded)")));
	let mut current = shot
		.loaded
		.iter()
		.position(|(id, ..)| *id == graph.selected)
		.unwrap_or(0);
	ui.set_next_item_width(280.0);
	if ui.combo_simple_string("##graph", &mut current, &labels) {
		if current < first_unloaded {
			graph.selected = shot.loaded[current].0;
		} else if let Some((id, path)) = shot.unloaded.get(current - first_unloaded) {
			load_project_graph(graph, project, ecs, *id, path);
		}
		graph.reset();
	}
	ui.same_line();
	let file = shot
		.loaded
		.iter()
		.find(|(id, ..)| *id == graph.selected)
		.and_then(|(_, _, file)| file.clone());
	if ui.button(if graph.dirty { "Save *" } else { "Save" }) {
		match &file {
			Some(path) => save_to(graph, project, path),
			None => {
				ui.open_popup(SAVE_AS);
			}
		}
	}
	ui.same_line();
	if ui.button("Save As...") {
		graph.save_as_name.clear();
		ui.open_popup(SAVE_AS);
	}
	ui.same_line();
	let mut preview = scene.preview_graph == Some(graph.selected);
	if ui.checkbox("Show in Viewport", &mut preview) {
		scene.preview_graph = preview.then_some(graph.selected);
	}
	ui.same_line();
	let mut main = shot.main == Some(graph.selected);
	if ui.checkbox("Main graph", &mut main) {
		let id = main.then_some(graph.selected);
		ecs.resource_mut::<Renderer>().graphs_mut().set_main(id);
	}
	save_as_popup(ui, graph, project, ecs);
}

fn load_project_graph(
	graph: &mut GraphEditor,
	project: &Project,
	ecs: &mut bevy_ecs::world::World,
	id: GraphId,
	path: &ZPath,
) {
	let loaded = {
		let registry = ecs.resource::<AssetRegistry>();
		GraphDesc::load(project, registry, id.0)
	};
	match loaded {
		Ok(desc) => {
			ecs.resource_mut::<Renderer>().set_graph(desc);
			graph.selected = id;
			graph.notice = None;
		}
		Err(e) => graph.notice = Some(format!("{path} cannot be read: {e}")),
	}
}

fn save_to(graph: &mut GraphEditor, project: &Project, path: &ZPath) {
	let Some(desc) = &graph.working else { return };
	match desc.to_text().and_then(|text| project.write(path, text.as_bytes())) {
		Ok(()) => {
			graph.dirty = false;
			graph.notice = Some(format!("Saved {path}"));
		}
		Err(e) => graph.notice = Some(format!("{path} is not saved: {e}")),
	}
}

/// "Save As": a copy with an id of its own goes into `Assets/Graphs` and is
/// the graph that is edited from then on.
fn save_as_popup(ui: &Ui, graph: &mut GraphEditor, project: &Project, ecs: &mut bevy_ecs::world::World) {
	let Some(_token) = ui.begin_popup(SAVE_AS) else {
		return;
	};
	ui.text("A copy of this graph goes into Assets/Graphs.");
	ui.input_text("Name", &mut graph.save_as_name).build();
	if ui.button("Save") && !graph.save_as_name.trim().is_empty() {
		let name = graph.save_as_name.trim().to_string();
		let result = (|| -> zenith_error::ZResult<GraphId> {
			let desc = graph
				.working
				.as_ref()
				.ok_or_else(|| zenith_error::ZenithError::Other("there is no graph".into()))?;
			let copy = copy_graph(desc, &name)?;
			let dir = ZPath::new(GRAPHS_DIR)?;
			if !project.exists(&dir)? {
				project.create_dir(&dir)?;
			}
			let path = ZPath::new(&format!("{GRAPHS_DIR}/{}.zenithgraph", file_stem(&name)))?;
			project.write(&path, copy.to_text()?.as_bytes())?;
			let id = GraphId(copy.uuid);
			ecs.resource_mut::<Renderer>().set_graph(copy);
			Ok(id)
		})();
		match result {
			Ok(id) => {
				graph.selected = id;
				graph.reset();
				graph.notice = Some(format!("Saved as {name}"));
				// The list of assets learns about the new file.
				let registry = AssetRegistry::scan(project);
				if let Ok(registry) = registry {
					ecs.insert_resource(registry);
				}
				ui.close_current_popup();
			}
			Err(e) => graph.notice = Some(format!("The graph is not saved: {e}")),
		}
	}
	ui.same_line();
	if ui.button("Cancel") {
		ui.close_current_popup();
	}
}

/// The node editor itself.
fn nodes(
	ui: &Ui,
	graph: &mut GraphEditor,
	desc: &GraphDesc,
	shot: &Snapshot,
	edits: &mut Vec<Edit>,
	positions: &mut HashMap<i32, [f32; 2]>,
) {
	let place = std::mem::take(&mut graph.place_nodes);
	let pending = std::mem::take(&mut graph.pending);
	let Some((context, editor_context)) = &graph.imnodes else {
		ui.text_disabled("The node editor is starting");
		return;
	};
	let marked: Vec<NodeId> = shot
		.diagnostics
		.iter()
		.filter(|diagnostic| diagnostic.severity == Severity::Error)
		.flat_map(|diagnostic| diagnostic.node.into_iter().chain(diagnostic.related.iter().copied()))
		.collect();

	let setup = ui.imnodes(context).editor(Some(editor_context));
	setup.enable_link_detach_with_ctrl();
	setup.enable_multiple_select_with_shift();
	setup.style_colors_dark();
	if place {
		for node in &desc.nodes {
			if let Some(place) = &node.editor {
				setup.set_node_pos_grid(ImNode::new(node.id.0 as i32), place.pos);
			}
		}
	}
	for (node, at) in pending {
		setup.set_node_pos_screen(ImNode::new(node.0 as i32), at);
	}
	let canvas = setup.begin_nodes();

	for node in &desc.nodes {
		let pass = shot.passes.get(&node.pass);
		let errored = marked.contains(&node.id) || pass.is_none();
		let title = pass.map_or_else(|| format!("{} ?", node.pass), |pass| pass.display_name.to_string());
		let color = if errored {
			ERROR_COLOR
		} else {
			pass.map_or(ERROR_COLOR, |pass| category_color(pass.category))
		};
		let _colors = [
			canvas.push_color(ColorElement::TitleBar, color),
			canvas.push_color(ColorElement::TitleBarHovered, lighter(color)),
			canvas.push_color(ColorElement::TitleBarSelected, lighter(color)),
		];
		let token = canvas.node(ImNode::new(node.id.0 as i32));
		token.title_bar(|| ui.text(&title));
		let Some(pass) = pass else {
			let pin = token.static_attr(PinId::new(pin_id(node.id, PARAM_SLOT, false)));
			ui.text_disabled("This pass is not known");
			pin.end();
			token.end();
			continue;
		};
		for (index, input) in pass.inputs.iter().enumerate() {
			let _pin_color = canvas.push_color(ColorElement::Pin, kind_color(&input.kind));
			let pin = token.input_attr(PinId::new(pin_id(node.id, index, false)), PinShape::CircleFilled);
			ui.text(input.name.as_ref());
			pin.end();
		}
		for (index, output) in pass.outputs.iter().enumerate() {
			let _pin_color = canvas.push_color(ColorElement::Pin, kind_color(&output.kind));
			let pin = token.output_attr(PinId::new(pin_id(node.id, index, true)), PinShape::QuadFilled);
			ui.text(output.name.as_ref());
			pin.end();
		}
		for (index, param) in pass.params.iter().enumerate().take(PARAM_SLOT) {
			let pin = token.static_attr(PinId::new(pin_id(node.id, PARAM_SLOT + index, false)));
			let _id = ui.push_id(format!("{}-{}", node.id.0, param.name).as_str());
			let current = param_value(node, param);
			if let Some(value) = param_widget(ui, param, &current) {
				edits.push(Edit::Param {
					node: node.id,
					param: param.clone(),
					value,
				});
			}
			pin.end();
		}
		token.end();
	}

	for (index, link) in desc.links.iter().enumerate() {
		if let Some((from, to)) = link_pins(desc, &shot.passes, link) {
			canvas.link(LinkId::new(index as i32 + 1), PinId::new(from), PinId::new(to));
		}
	}
	canvas.minimap(0.15, imnodes::MiniMapLocation::BottomRight);
	let post = canvas.end();

	// Where the nodes are now, for the file.
	let ini = post.save_state_to_ini_string();
	*positions = parse_positions(&ini);

	if let Some(created) = post.is_link_created() {
		match link_ends(desc, &shot.passes, created.start_attr.raw(), created.end_attr.raw()) {
			Ok((from, to)) => edits.push(Edit::Link(from, to)),
			Err(why) => graph.notice = Some(why),
		}
	}
	if let Some(destroyed) = post.is_link_destroyed()
		&& let Some(link) = desc.links.get((destroyed.raw() - 1).max(0) as usize)
	{
		edits.push(Edit::Unlink(link.to.clone()));
	}
	if ui.is_window_focused() && ui.is_key_pressed(Key::Delete) {
		for link in post.selected_links() {
			if let Some(link) = desc.links.get((link.raw() - 1).max(0) as usize) {
				edits.push(Edit::Unlink(link.to.clone()));
			}
		}
		let doomed: Vec<NodeId> = post
			.selected_nodes()
			.into_iter()
			.map(|node| NodeId(node.raw().max(0) as u32))
			.collect();
		if !doomed.is_empty() {
			edits.push(Edit::Remove(doomed));
		}
	}
	let over_empty = post.is_editor_hovered() && post.hovered_node().is_none() && post.hovered_link().is_none();
	if over_empty && ui.is_mouse_clicked(MouseButton::Right) {
		graph.menu_at = ui.mouse_pos();
		ui.open_popup(ADD_MENU);
	}
	add_menu(ui, graph, &shot.passes, edits);
}

fn lighter(color: [f32; 4]) -> [f32; 4] {
	[
		(color[0] + 0.15).min(1.0),
		(color[1] + 0.15).min(1.0),
		(color[2] + 0.15).min(1.0),
		color[3],
	]
}

/// The pins a link joins, if its passes and slots are known.
fn link_pins(desc: &GraphDesc, passes: &PassRegistry, link: &zenith_renderer::graph::LinkDesc) -> Option<(i32, i32)> {
	let out_pass = passes.get(&desc.node(link.from.0)?.pass)?;
	let in_pass = passes.get(&desc.node(link.to.0)?.pass)?;
	let out = out_pass.outputs.iter().position(|slot| slot.name == link.from.1)?;
	let input = in_pass.inputs.iter().position(|slot| slot.name == link.to.1)?;
	Some((pin_id(link.from.0, out, true), pin_id(link.to.0, input, false)))
}

/// The menu of right click: every pass that can be added, by category.
fn add_menu(ui: &Ui, graph: &GraphEditor, passes: &PassRegistry, edits: &mut Vec<Edit>) {
	let Some(_popup) = ui.begin_popup(ADD_MENU) else {
		return;
	};
	ui.text_disabled("Add a node");
	ui.separator();
	let categories = [
		(PassCategory::Input, "Input"),
		(PassCategory::Geometry, "Geometry"),
		(PassCategory::Lighting, "Lighting"),
		(PassCategory::Post, "Post"),
		(PassCategory::Output, "Output"),
		(PassCategory::Debug, "Debug"),
	];
	for (category, label) in categories {
		let mut of: Vec<&PassDesc> = passes.iter().filter(|pass| pass.category == category).collect();
		if of.is_empty() {
			continue;
		}
		of.sort_by(|a, b| a.display_name.cmp(&b.display_name));
		ui.menu(label, || {
			for pass in of {
				if ui.menu_item(pass.display_name.as_ref()) {
					edits.push(Edit::Add {
						pass: pass.type_name.to_string(),
						at: graph.menu_at,
					});
				}
			}
		});
	}
}

/// The widget of a parameter, in a node. `Some` if the user changed it.
fn param_widget(ui: &Ui, param: &ParamDesc, current: &ParamValue) -> Option<ParamValue> {
	let label = format!("{}##value", param.name);
	ui.set_next_item_width(PARAM_WIDTH);
	let changed = match (&param.ty, current) {
		(ParamType::Bool, ParamValue::Bool(value)) => {
			let mut value = *value;
			ui.checkbox(&label, &mut value).then_some(ParamValue::Bool(value))
		}
		(ParamType::Int, ParamValue::Int(value)) => {
			let mut number = *value as i32;
			let used = match (param.min, param.max) {
				(Some(min), Some(max)) => ui.slider_i32(&label, &mut number, min as i32, max as i32),
				_ => ui.input_int(&label, &mut number),
			};
			used.then(|| ParamValue::Int(i64::from(number)))
		}
		(ParamType::Float, ParamValue::Float(value)) => {
			let mut number = *value;
			let used = match (param.min, param.max) {
				(Some(min), Some(max)) => ui.slider_f32(&label, &mut number, min as f32, max as f32),
				_ => ui.drag_float(&label, &mut number),
			};
			used.then_some(ParamValue::Float(number))
		}
		(ParamType::Vec2, ParamValue::Vec2(value)) => {
			let mut value = *value;
			ui.input_float2(&label, &mut value)
				.build()
				.then_some(ParamValue::Vec2(value))
		}
		(ParamType::Vec3, ParamValue::Vec3(value)) => {
			let mut value = *value;
			ui.input_float3(&label, &mut value)
				.build()
				.then_some(ParamValue::Vec3(value))
		}
		(ParamType::Vec4, ParamValue::Vec4(value)) => {
			let mut value = *value;
			ui.input_float4(&label, &mut value)
				.build()
				.then_some(ParamValue::Vec4(value))
		}
		(ParamType::Color, ParamValue::Color(value)) => {
			let mut value = *value;
			ui.color_edit4(&label, &mut value).then_some(ParamValue::Color(value))
		}
		(ParamType::Enum(variants), ParamValue::Enum(value)) => {
			let names: Vec<&str> = variants.iter().map(AsRef::as_ref).collect();
			let mut index = names.iter().position(|name| name == value).unwrap_or(0);
			ui.combo_simple_string(&label, &mut index, &names)
				.then(|| ParamValue::Enum(names[index].to_string()))
		}
		(ParamType::String, ParamValue::String(value)) => {
			let mut text = value.clone();
			ui.input_text(&label, &mut text)
				.build()
				.then_some(ParamValue::String(text))
		}
		_ => {
			ui.text_disabled(&param.name);
			None
		}
	};
	if ui.is_item_hovered() && !param.tooltip.is_empty() {
		ui.tooltip_text(param.tooltip.as_ref());
	}
	changed
}

/// Puts what the user did into the working copy. `true` if the graph changed
/// in a way that has to be compiled again.
fn apply(desc: &mut GraphDesc, passes: &PassRegistry, edits: Vec<Edit>, graph: &mut GraphEditor) -> bool {
	let mut changed = false;
	for edit in edits {
		changed = true;
		match edit {
			Edit::Param { node, param, value } => {
				if let Some(node) = desc.node_mut(node) {
					set_param(node, &param, &value);
				}
			}
			Edit::Link(from, to) => desc.connect(from, to),
			Edit::Unlink(to) => desc.disconnect(&to),
			Edit::Remove(nodes) => {
				for node in nodes {
					desc.remove_node(node);
				}
			}
			Edit::Add { pass, at } => {
				if passes.get(&pass).is_some() {
					let id = desc.add_node(pass, [0.0, 0.0]);
					graph.pending.push((id, at));
				}
			}
		}
	}
	changed
}

#[cfg(test)]
mod tests {
	use dear_imgui_wgpu::{WgpuInitInfo, WgpuRenderer};
	use zenith_filesystem::Project as FsProject;
	use zenith_renderer::wgpu;

	use super::*;
	use crate::project::OpenProject;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	/// A GPU without a window; `None` where there is no adapter.
	fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
		pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
			label: Some("test device"),
			required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
			..Default::default()
		}))
		.ok()
	}

	/// An editor that has a world with a renderer in it.
	fn editor_with_renderer(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
	) -> Result<(tempfile::TempDir, Editor), Box<dyn std::error::Error>> {
		let tmp = tempfile::tempdir()?;
		let path = tmp.path().join("Game");
		FsProject::create(&path)?;
		let mut project = OpenProject::open(&path)?;
		project.create_world("", "A")?;
		let mut editor = Editor::new(None);
		editor.scene.draw(device, queue, &mut project, "A").ok_or("not drawn")?;
		editor.project = Some(project);
		editor.active_world = Some("A".to_string());
		Ok((tmp, editor))
	}

	fn world_of(editor: &mut Editor) -> Result<&mut bevy_ecs::world::World, Box<dyn std::error::Error>> {
		Ok(editor
			.project
			.as_mut()
			.ok_or("no project")?
			.worlds
			.get_mut("A")
			.ok_or("no A")?
			.world
			.world_mut())
	}

	#[test]
	fn the_panel_works_on_the_graphs_of_the_renderer() -> TestResult {
		// There is one UI context at a time.
		let _one_context = crate::panels::UI_LOCK
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		let Some((device, queue)) = gpu() else {
			return Ok(());
		};
		let (_tmp, mut editor) = editor_with_renderer(&device, &queue)?;

		// The UI with no window: a renderer behind it makes frames possible.
		let mut context = dear_imgui_rs::Context::create();
		let _ = context.set_ini_filename(None::<std::path::PathBuf>);
		let mut backend = WgpuRenderer::new(
			WgpuInitInfo::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb),
			&mut context,
		)?;
		editor.graph.init(&context);
		for _ in 0..3 {
			context.io_mut().set_display_size([1280.0, 720.0]);
			context.io_mut().set_delta_time(1.0 / 60.0);
			let frame = context.try_begin_frame()?;
			draw(&mut editor, frame.ui());
			let pending = frame.render(backend.renderer_consumer()?);
			drop(backend.reconcile_frame(pending)?);
		}
		let working = editor.graph.working.clone().ok_or("no graph was loaded")?;
		assert_eq!(working.uuid, GraphId::BUILTIN.0, "the built-in graph is the first");
		assert!(
			working.nodes.iter().all(|node| node.editor.is_some()),
			"every node has a place"
		);

		// Edits: a parameter changes, a node comes, a needed link goes and
		// comes back.
		let passes = world_of(&mut editor)?.resource::<Renderer>().passes().clone();
		let mut desc = working;
		let mut graph = GraphEditor::default();
		let tonemap = desc
			.nodes
			.iter()
			.find(|n| n.pass == "zenith.tonemap")
			.ok_or("no tonemap")?
			.id;
		let exposure = passes
			.get("zenith.tonemap")
			.and_then(|pass| pass.params.iter().find(|p| p.name == "exposure").cloned())
			.ok_or("no exposure")?;
		let edit = Edit::Param {
			node: tonemap,
			param: exposure,
			value: ParamValue::Float(2.0),
		};
		assert!(apply(&mut desc, &passes, vec![edit], &mut graph));
		assert!(desc.node(tonemap).is_some_and(|n| n.params.contains_key("exposure")));

		let before = desc.nodes.len();
		let add = Edit::Add {
			pass: "zenith.sky".into(),
			at: [10.0, 10.0],
		};
		assert!(apply(&mut desc, &passes, vec![add], &mut graph));
		assert_eq!(desc.nodes.len(), before + 1);
		assert_eq!(graph.pending.len(), 1, "it is to be put where the menu was");

		let output = desc
			.nodes
			.iter()
			.find(|n| n.pass == "zenith.output")
			.ok_or("no output")?
			.id;
		assert!(apply(
			&mut desc,
			&passes,
			vec![Edit::Unlink(SlotRef::new(output, "color"))],
			&mut graph
		));
		let ecs = world_of(&mut editor)?;
		ecs.resource_mut::<Renderer>().set_graph(desc.clone());
		let diagnostics = |ecs: &bevy_ecs::world::World| {
			ecs.resource::<Renderer>()
				.graphs()
				.get(GraphId::BUILTIN)
				.map(|entry| entry.diagnostics.clone())
				.unwrap_or_default()
		};
		assert!(
			diagnostics(ecs).iter().any(|d| d.severity == Severity::Error),
			"{:?}",
			diagnostics(ecs)
		);
		let link = Edit::Link(SlotRef::new(tonemap, "color"), SlotRef::new(output, "color"));
		assert!(apply(&mut desc, &passes, vec![link], &mut graph));
		ecs.resource_mut::<Renderer>().set_graph(desc);
		assert!(
			!diagnostics(ecs).iter().any(|d| d.severity == Severity::Error),
			"{:?}",
			diagnostics(ecs)
		);

		// A graph saved as goes into the project and is found by its id.
		let files = std::sync::Arc::clone(&editor.project.as_ref().ok_or("no project")?.project);
		let copy = copy_graph(&editor.graph.working.clone().ok_or("no graph")?, "Mine")?;
		files.create_dir(&ZPath::new(GRAPHS_DIR)?)?;
		let path = ZPath::new(&format!("{GRAPHS_DIR}/Mine.zenithgraph"))?;
		files.write(&path, copy.to_text()?.as_bytes())?;
		let registry = AssetRegistry::scan(&files)?;
		let id = registry.id_of(&path).ok_or("the file is not an asset")?;
		assert_eq!(id, copy.uuid);
		assert_eq!(
			GraphDesc::load(&files, &registry, id)?,
			copy,
			"what was written is what is read"
		);

		editor.graph.shutdown();
		backend.shutdown(&mut context)?;
		Ok(())
	}
}
