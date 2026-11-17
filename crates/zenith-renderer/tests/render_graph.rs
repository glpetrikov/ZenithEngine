//! The render graph with passes made for the test: compiling needs no GPU;
//! running does, and is skipped where there is no adapter.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used, clippy::panic)]

use bevy_ecs::world::World;
use serde_json::{Map, json};
use zenith_error::{ZResult, ZenithError};
use zenith_renderer::{
	CacheSettings, GpuCache,
	assets::AssetId,
	graph::{
		CompiledGraph, ExecContext, Frame, GraphDesc, GraphId, GraphRegistry, InputDesc, NodeDesc, NodeId, OutputDesc,
		ParamDesc, ParamMap, PassAction, PassCategory, PassDesc, PassRegistry, PrepareContext, RenderPass, Severity,
		SlotKind, SlotRef, TextureDesc, TexturePool, ViewInfo, ViewKey, compile,
		desc::LinkDesc,
		pass::{FormatRule, SizeRule},
	},
	wgpu,
};

const NUMBER: &str = "number";
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A texture of the graph, the size of the view.
const PLAIN: TextureDesc = TextureDesc {
	format: FormatRule::Fixed(FORMAT),
	size: SizeRule::View { scale: 1.0 },
};

fn failed(what: &str) -> ZenithError {
	ZenithError::Other(format!("{what} is not bound"))
}

/// Publishes a number.
#[derive(Default)]
struct Input {
	value: f32,
}

impl RenderPass for Input {
	fn describe() -> PassDesc {
		PassDesc::new("test.input", "Input", PassCategory::Input)
			.output(OutputDesc::data("value", NUMBER))
			.param(ParamDesc::float("value", 1.0))
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.value = params.float("value").unwrap_or(1.0);
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		ctx.set_data("value", self.value);
		PassAction::Run
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

/// Creates a texture and clears it to green times the number it is given;
/// skips when that is 0.
#[derive(Default)]
struct Create;

impl RenderPass for Create {
	fn describe() -> PassDesc {
		PassDesc::new("test.create", "Create", PassCategory::Geometry)
			.input(InputDesc::read("value", SlotKind::data(NUMBER)).optional())
			.output(OutputDesc::create("color", SlotKind::Color, PLAIN))
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		if ctx.data::<f32>("value") == Some(&0.0) {
			PassAction::Skip
		} else {
			PassAction::Run
		}
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		let green = f64::from(ctx.data::<f32>("value").copied().unwrap_or(1.0));
		let clear = wgpu::Color {
			r: 0.0,
			g: green,
			b: 0.0,
			a: 1.0,
		};
		ctx.begin_render_pass("create", &[("color", clear)], None)
			.ok_or_else(|| failed("color"))
			.map(drop)
	}
}

/// Draws into a texture (nothing, here): a new version of it.
#[derive(Default)]
struct Draw;

impl RenderPass for Draw {
	fn describe() -> PassDesc {
		PassDesc::new("test.draw", "Draw", PassCategory::Geometry)
			.input(InputDesc::write("color", SlotKind::Color))
			.output(OutputDesc::pass_through("color", SlotKind::Color, "color"))
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		// Red, if the graph wrongly cleared it: it must be loaded.
		ctx.begin_render_pass("draw", &[("color", wgpu::Color::RED)], None)
			.ok_or_else(|| failed("color"))
			.map(drop)
	}
}

/// Reads a texture and makes another.
#[derive(Default)]
struct Read;

impl RenderPass for Read {
	fn describe() -> PassDesc {
		PassDesc::new("test.read", "Read", PassCategory::Post)
			.input(InputDesc::read("color", SlotKind::Color))
			.output(OutputDesc::create("out", SlotKind::Color, PLAIN))
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

/// Reads two textures into the target of the view.
#[derive(Default)]
struct Mix;

impl RenderPass for Mix {
	fn describe() -> PassDesc {
		PassDesc::new("test.mix", "Mix", PassCategory::Post)
			.input(InputDesc::read("a", SlotKind::Color))
			.input(InputDesc::read("b", SlotKind::Color).optional())
			.output(OutputDesc::create("color", SlotKind::Color, TextureDesc::VIEW_TARGET))
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

/// Copies a texture of the graph into the target of the view.
#[derive(Default)]
struct CopyToTarget;

impl RenderPass for CopyToTarget {
	fn describe() -> PassDesc {
		PassDesc::new("test.copy", "Copy", PassCategory::Post)
			.input(InputDesc::read("color", SlotKind::Color))
			.output(OutputDesc::create("target", SlotKind::Color, TextureDesc::VIEW_TARGET))
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		let from = ctx.texture("color").ok_or_else(|| failed("color"))?.texture();
		let to = ctx.texture("target").ok_or_else(|| failed("target"))?.texture();
		ctx.encoder
			.copy_texture_to_texture(from.as_image_copy(), to.as_image_copy(), from.size());
		Ok(())
	}
}

/// Clears the target of the view (the graph decides to what).
#[derive(Default)]
struct Target;

impl RenderPass for Target {
	fn describe() -> PassDesc {
		PassDesc::new("test.target", "Target", PassCategory::Geometry).output(OutputDesc::create(
			"color",
			SlotKind::Color,
			TextureDesc::VIEW_TARGET,
		))
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		ctx.begin_render_pass("target", &[("color", wgpu::Color::BLUE)], None)
			.ok_or_else(|| failed("color"))
			.map(drop)
	}
}

#[derive(Default)]
struct Output;

impl RenderPass for Output {
	fn describe() -> PassDesc {
		PassDesc::new("test.output", "Output", PassCategory::Output).input(InputDesc::read("color", SlotKind::Color))
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

fn passes() -> PassRegistry {
	let mut passes = PassRegistry::new();
	passes.register::<Input>();
	passes.register::<Create>();
	passes.register::<Draw>();
	passes.register::<Read>();
	passes.register::<Mix>();
	passes.register::<CopyToTarget>();
	passes.register::<Target>();
	passes.register::<Output>();
	passes
}

/// A graph of `nodes` (id, pass) and `links` ((id, output), (id, input)).
fn graph(nodes: &[(u32, &str)], links: &[((u32, &str), (u32, &str))]) -> GraphDesc {
	let mut graph = GraphDesc::new(AssetId::generate().expect("an id"), "test");
	graph.nodes = nodes
		.iter()
		.map(|&(id, pass)| NodeDesc {
			id: NodeId(id),
			pass: pass.into(),
			params: Map::new(),
			editor: None,
			extra: Map::new(),
		})
		.collect();
	graph.links = links
		.iter()
		.map(|&((from, output), (to, input))| LinkDesc {
			from: SlotRef::new(NodeId(from), output),
			to: SlotRef::new(NodeId(to), input),
		})
		.collect();
	graph
}

fn ids(order: &[u32]) -> Vec<NodeId> {
	order.iter().map(|&id| NodeId(id)).collect()
}

fn errors(result: Result<CompiledGraph, Vec<zenith_renderer::graph::Diagnostic>>) -> Vec<String> {
	match result {
		Ok(graph) => panic!("it compiled: {graph:?}"),
		Err(diagnostics) => diagnostics
			.into_iter()
			.filter(|d| d.severity == Severity::Error)
			.map(|d| d.to_string())
			.collect(),
	}
}

#[test]
fn a_pass_that_reads_a_texture_comes_before_the_one_that_draws_over_it() {
	// 3 reads what 1 created; 2 draws over it. Without that rule the ties
	// would put 2 (the smaller id) before 3.
	let desc = graph(
		&[
			(1, "test.create"),
			(2, "test.draw"),
			(3, "test.read"),
			(5, "test.mix"),
			(6, "test.output"),
		],
		&[
			((1, "color"), (2, "color")),
			((1, "color"), (3, "color")),
			((2, "color"), (5, "a")),
			((3, "out"), (5, "b")),
			((5, "color"), (6, "color")),
		],
	);
	let compiled = compile(&desc, &passes()).expect("it compiles");
	assert_eq!(compiled.order(), ids(&[1, 3, 2, 5, 6]));
	assert_eq!(compiled.diagnostics(), &[]);
}

#[test]
fn what_cannot_run_is_left_out_with_a_warning() {
	let desc = graph(
		&[
			(1, "test.target"),
			(2, "test.output"),
			(3, "plugin.fog"),
			(4, "test.draw"),
			(5, "test.create"),
		],
		&[((1, "color"), (2, "color")), ((3, "color"), (4, "color"))],
	);
	let compiled = compile(&desc, &passes()).expect("it compiles");
	assert_eq!(compiled.order(), ids(&[1, 2]), "5 is left out too: nothing reads it");
	let warned: Vec<Option<NodeId>> = compiled.diagnostics().iter().map(|d| d.node).collect();
	assert_eq!(
		warned,
		[Some(NodeId(3)), Some(NodeId(4))],
		"{:?}",
		compiled.diagnostics()
	);
}

#[test]
fn a_link_between_different_kinds_is_dropped() {
	let desc = graph(
		&[
			(1, "test.input"),
			(2, "test.create"),
			(3, "test.mix"),
			(4, "test.output"),
		],
		&[
			((2, "color"), (3, "a")),
			((1, "value"), (3, "b")),
			((3, "color"), (4, "color")),
		],
	);
	let compiled = compile(&desc, &passes()).expect("it compiles without that link");
	assert_eq!(compiled.order(), ids(&[2, 3, 4]));
	assert!(
		compiled
			.diagnostics()
			.iter()
			.any(|d| d.message.contains("the link is dropped")),
		"{:?}",
		compiled.diagnostics()
	);
}

#[test]
fn a_broken_graph_says_why() {
	let passes = passes();
	let no_output = graph(&[(1, "test.target")], &[]);
	assert!(errors(compile(&no_output, &passes))[0].contains("no output node"));

	let cycle = graph(
		&[(1, "test.mix"), (2, "test.read"), (3, "test.output")],
		&[
			((1, "color"), (2, "color")),
			((2, "out"), (1, "a")),
			((1, "color"), (3, "color")),
		],
	);
	assert!(errors(compile(&cycle, &passes))[0].contains("cycle"));

	let two_writers = graph(
		&[
			(1, "test.create"),
			(2, "test.draw"),
			(3, "test.draw"),
			(4, "test.mix"),
			(5, "test.output"),
		],
		&[
			((1, "color"), (2, "color")),
			((1, "color"), (3, "color")),
			((2, "color"), (4, "a")),
			((3, "color"), (4, "b")),
			((4, "color"), (5, "color")),
		],
	);
	assert!(errors(compile(&two_writers, &passes))[0].contains("drawn into by"));

	let not_the_target = graph(
		&[(1, "test.create"), (2, "test.output")],
		&[((1, "color"), (2, "color"))],
	);
	assert!(errors(compile(&not_the_target, &passes))[0].contains("not the target of the view"));
}

#[test]
fn textures_that_are_not_used_at_once_share_one() {
	// 1.color is done when 3 makes 3.out: they share; 2.out overlaps both.
	let desc = graph(
		&[
			(1, "test.create"),
			(2, "test.read"),
			(3, "test.read"),
			(4, "test.mix"),
			(5, "test.output"),
		],
		&[
			((1, "color"), (2, "color")),
			((2, "out"), (3, "color")),
			((3, "out"), (4, "a")),
			((4, "color"), (5, "color")),
		],
	);
	let compiled = compile(&desc, &passes()).expect("it compiles");
	assert_eq!(compiled.physical_textures(), 2);
}

#[test]
fn parameters_are_read_from_the_file() {
	let mut desc = graph(
		&[
			(1, "test.input"),
			(2, "test.create"),
			(3, "test.copy"),
			(4, "test.output"),
		],
		&[
			((1, "value"), (2, "value")),
			((2, "color"), (3, "color")),
			((3, "target"), (4, "color")),
		],
	);
	if let Some(node) = desc.node_mut(NodeId(1)) {
		node.params.insert("value".into(), json!(0.25));
		node.params.insert("shiny".into(), json!(true));
	}
	let compiled = compile(&desc, &passes()).expect("it compiles");
	assert_eq!(compiled.params(NodeId(1)).and_then(|p| p.float("value")), Some(0.25));
	assert_eq!(compiled.diagnostics().len(), 1, "a parameter it does not have");
}

#[test]
fn a_graph_that_stops_compiling_keeps_running_its_last_version() {
	let passes = passes();
	let mut graphs = GraphRegistry::new();
	let good = graph(
		&[(1, "test.target"), (2, "test.output")],
		&[((1, "color"), (2, "color"))],
	);
	let id = GraphId(good.uuid);
	assert_eq!(graphs.set(good.clone(), &passes), &[]);

	let mut broken = good;
	broken.links.clear();
	assert!(!graphs.set(broken, &passes).is_empty(), "it says why");
	assert!(
		graphs.get(id).is_some_and(|entry| entry.compiled.is_some()),
		"the old one runs"
	);
	assert_eq!(graphs.resolve(Some(id)).map(|(found, _)| found), Some(id));

	let unknown = GraphId(AssetId::generate().expect("an id"));
	assert!(
		graphs.resolve(Some(unknown)).is_none(),
		"no main graph and no built-in one yet"
	);
	graphs.set_main(Some(id));
	assert_eq!(
		graphs.resolve(Some(unknown)).map(|(found, _)| found),
		Some(id),
		"the main graph"
	);
}

// --- Running ----------------------------------------------------------------

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
	let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
	let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
	pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
		label: Some("test device"),
		required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
		..Default::default()
	}))
	.ok()
}

/// What a test draws with: the GPU, a cache, a pool, a target of 4x4.
struct Gpu {
	device: wgpu::Device,
	queue: wgpu::Queue,
	cache: GpuCache,
	pool: TexturePool,
	world: World,
	target: wgpu::Texture,
	view: wgpu::TextureView,
	frame: u64,
}

impl Gpu {
	fn new() -> Option<Self> {
		let Some((device, queue)) = device() else {
			eprintln!("no adapter: skipped");
			return None;
		};
		let target = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("target"),
			size: wgpu::Extent3d {
				width: 4,
				height: 4,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: FORMAT,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT
				| wgpu::TextureUsages::COPY_SRC
				| wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		let view = target.create_view(&wgpu::TextureViewDescriptor::default());
		Some(Self {
			cache: GpuCache::new(&device, &queue, CacheSettings::default()),
			device,
			queue,
			pool: TexturePool::new(),
			world: World::new(),
			target,
			view,
			frame: 0,
		})
	}

	/// Runs `graph` into the target, which the first camera clears to
	/// `clear`; returns the top left pixel.
	fn run(&mut self, graph: &mut CompiledGraph, clear: Option<wgpu::Color>) -> [u8; 4] {
		self.frame += 1;
		let mut encoder = self
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		let view = ViewInfo {
			key: ViewKey(1),
			target: &self.view,
			target_format: FORMAT,
			target_size: (4, 4),
			viewport: None,
			clear,
		};
		let mut frame = Frame {
			device: &self.device,
			queue: &self.queue,
			cache: &mut self.cache,
			world: &self.world,
			pool: &mut self.pool,
			encoder: &mut encoder,
			number: self.frame,
		};
		graph.run(&mut frame, &view);
		self.pool.end_frame(self.frame);
		self.queue.submit([encoder.finish()]);
		self.read()
	}

	fn read(&self) -> [u8; 4] {
		let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("read back"),
			size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
			usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
			mapped_at_creation: false,
		});
		let mut encoder = self
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		encoder.copy_texture_to_buffer(
			self.target.as_image_copy(),
			wgpu::TexelCopyBufferInfo {
				buffer: &buffer,
				layout: wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
					rows_per_image: Some(1),
				},
			},
			wgpu::Extent3d {
				width: 1,
				height: 1,
				depth_or_array_layers: 1,
			},
		);
		self.queue.submit([encoder.finish()]);
		buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("map the buffer"));
		self.device
			.poll(wgpu::PollType::wait_indefinitely())
			.expect("wait for the GPU");
		let bytes = buffer.get_mapped_range(..4).expect("read the buffer");
		[bytes[0], bytes[1], bytes[2], bytes[3]]
	}

	/// Fills the target with `rgba` outside any graph.
	fn fill(&self, color: wgpu::Color) {
		let mut encoder = self
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("fill"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &self.view,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(color),
					store: wgpu::StoreOp::Store,
				},
			})],
			depth_stencil_attachment: None,
			occlusion_query_set: None,
			timestamp_writes: None,
			multiview_mask: None,
		}));
		self.queue.submit([encoder.finish()]);
	}
}

#[test]
fn the_target_is_cleared_only_by_the_first_camera_on_it() {
	let Some(mut gpu) = Gpu::new() else {
		return;
	};
	let desc = graph(
		&[(1, "test.target"), (2, "test.draw"), (3, "test.output")],
		&[((1, "color"), (2, "color")), ((2, "color"), (3, "color"))],
	);
	let mut compiled = compile(&desc, &passes()).expect("it compiles");
	assert_eq!(
		gpu.run(&mut compiled, Some(wgpu::Color::GREEN)),
		[0, 255, 0, 255],
		"to the color of the camera; the pass that draws over it loads it"
	);
	gpu.fill(wgpu::Color::WHITE);
	assert_eq!(
		gpu.run(&mut compiled, None),
		[255; 4],
		"a second camera draws over the first"
	);
}

#[test]
fn data_and_textures_go_from_pass_to_pass() {
	let Some(mut gpu) = Gpu::new() else {
		return;
	};
	let mut desc = graph(
		&[
			(1, "test.input"),
			(2, "test.create"),
			(3, "test.copy"),
			(4, "test.output"),
		],
		&[
			((1, "value"), (2, "value")),
			((2, "color"), (3, "color")),
			((3, "target"), (4, "color")),
		],
	);
	if let Some(node) = desc.node_mut(NodeId(1)) {
		node.params.insert("value".into(), json!(0.5));
	}
	let mut compiled = compile(&desc, &passes()).expect("it compiles");
	let [r, g, b, a] = gpu.run(&mut compiled, Some(wgpu::Color::BLACK));
	assert_eq!((r, b, a), (0, 0, 255));
	assert!((127..=128).contains(&g), "green {g}: half, from the input node");
	assert_eq!(gpu.pool.len(), 1, "one texture of the graph");

	gpu.run(&mut compiled, Some(wgpu::Color::BLACK));
	assert_eq!(gpu.pool.len(), 1, "taken again from the pool");
}

#[test]
fn what_a_skipped_pass_creates_is_cleared() {
	let Some(mut gpu) = Gpu::new() else {
		return;
	};
	let mut desc = graph(
		&[
			(1, "test.input"),
			(2, "test.create"),
			(3, "test.copy"),
			(4, "test.output"),
		],
		&[
			((1, "value"), (2, "value")),
			((2, "color"), (3, "color")),
			((3, "target"), (4, "color")),
		],
	);
	if let Some(node) = desc.node_mut(NodeId(1)) {
		node.params.insert("value".into(), json!(0.0));
	}
	let mut compiled = compile(&desc, &passes()).expect("it compiles");
	assert_eq!(
		gpu.run(&mut compiled, Some(wgpu::Color::WHITE)),
		[0; 4],
		"transparent black, copied over the target"
	);
}
