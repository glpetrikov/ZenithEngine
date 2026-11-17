//! The input nodes: what a view is drawn from, given to the passes through
//! data slots.
//!
//! `zenith.input_camera` and `zenith.input_meshes` for now; the lights, the
//! environment and the sprites come with the passes that need them.
//!
//! The input nodes are the only nodes of the engine that read the world.

use zenith_error::ZResult;

use crate::{
	extract::{ExtractedLights, ExtractedViews},
	graph::{
		ExecContext, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc, PrepareContext, RenderPass,
		pass::{OutputDesc, data},
	},
	prepare::{CameraUniform, DrawLists, LightsUniform, MeshList, PreparedViews, SpriteList, SpriteLists},
};

/// The data of `zenith.input_camera`: the view the graph runs for.
#[derive(Clone, Debug)]
pub struct CameraData {
	/// The uniform buffer of the view ([`CameraUniform`]).
	pub buffer: wgpu::Buffer,
	/// The same on the CPU.
	pub uniform: CameraUniform,
}

/// The data of `zenith.input_meshes`: the batches of one phase.
#[derive(Clone, Debug)]
pub struct MeshesData {
	pub list: MeshList,
	/// The buffer the batches count their instances in (slot 1 of the
	/// vertex buffers). `None` when nothing is drawn.
	pub instances: Option<wgpu::Buffer>,
}

/// Gives the camera of the view.
#[derive(Default)]
pub struct InputCamera;

impl RenderPass for InputCamera {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.input_camera", "Camera", PassCategory::Input)
			.output(OutputDesc::data("camera", data::CAMERA))
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		let Some(view) = ctx
			.world
			.get_resource::<PreparedViews>()
			.and_then(|views| views.get(ctx.view.key))
		else {
			return PassAction::Skip;
		};
		let data = CameraData {
			buffer: view.buffer.clone(),
			uniform: view.uniform,
		};
		ctx.set_data("camera", data);
		PassAction::Run
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

/// Which meshes an input node gives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
	#[default]
	Opaque,
	Transparent,
}

/// Gives the meshes the view draws in one phase.
#[derive(Default)]
pub struct InputMeshes {
	phase: Phase,
}

impl RenderPass for InputMeshes {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.input_meshes", "Meshes", PassCategory::Input)
			.output(OutputDesc::data("meshes", data::MESHES))
			.param(
				ParamDesc::enumeration("phase", &["opaque", "transparent"], "opaque")
					.tooltip("Opaque meshes and masked ones, or the ones that blend"),
			)
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.phase = match params.str("phase") {
			Some("transparent") => Phase::Transparent,
			_ => Phase::Opaque,
		};
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		let Some(lists) = ctx.world.get_resource::<DrawLists>() else {
			return PassAction::Skip;
		};
		let Some(view) = lists.get(ctx.view.key) else {
			return PassAction::Skip;
		};
		let list = match self.phase {
			Phase::Opaque => view.opaque.clone(),
			Phase::Transparent => view.transparent.clone(),
		};
		let data = MeshesData {
			list,
			instances: lists.instance_buffer().cloned(),
		};
		ctx.set_data("meshes", data);
		PassAction::Run
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

/// The data of `zenith.input_lights`: the lights of the view.
#[derive(Clone, Debug)]
pub struct LightsData {
	/// The uniform buffer of the lights ([`LightsUniform`]).
	pub buffer: wgpu::Buffer,
	/// The same on the CPU (the shadow pass reads the light it fits to).
	pub uniform: LightsUniform,
}

/// What a view keeps between frames: its own buffer of lights.
#[derive(Default)]
struct ViewLights(Option<wgpu::Buffer>);

/// Gives the lights of the view, relative to its camera.
pub struct InputLights {
	ambient: [f32; 3],
}

impl Default for InputLights {
	fn default() -> Self {
		Self {
			ambient: DEFAULT_AMBIENT,
		}
	}
}

const DEFAULT_AMBIENT: [f32; 3] = [0.25, 0.25, 0.25];

impl RenderPass for InputLights {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.input_lights", "Lights", PassCategory::Input)
			.output(OutputDesc::data("lights", data::LIGHTS))
			.param(
				ParamDesc::color(
					"ambient",
					[DEFAULT_AMBIENT[0], DEFAULT_AMBIENT[1], DEFAULT_AMBIENT[2], 1.0],
				)
				.tooltip("Light that comes from everywhere, besides the environment"),
			)
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		let [r, g, b, _] = params.vec4("ambient").unwrap_or([0.25, 0.25, 0.25, 1.0]);
		self.ambient = [r, g, b];
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		let Some(position) = ctx
			.world
			.get_resource::<PreparedViews>()
			.and_then(|views| views.get(ctx.view.key))
			.map(|view| view.position)
		else {
			return PassAction::Skip;
		};
		let Some(lights) = ctx.world.get_resource::<ExtractedLights>() else {
			return PassAction::Skip;
		};
		let uniform = LightsUniform::build(lights, position, self.ambient);
		let device = ctx.device;
		let buffer = ctx
			.view_state::<ViewLights>()
			.0
			.get_or_insert_with(|| {
				device.create_buffer(&wgpu::BufferDescriptor {
					label: Some("Zenith Lights"),
					size: size_of::<LightsUniform>() as u64,
					usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
					mapped_at_creation: false,
				})
			})
			.clone();
		ctx.queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
		ctx.set_data("lights", LightsData { buffer, uniform });
		PassAction::Run
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

/// The data of `zenith.input_environment`: the sky of the view.
#[derive(Clone, Debug)]
pub struct EnvironmentData {
	/// The equirectangular picture, all of its mip levels. `None` while it is
	/// not loaded.
	pub texture: Option<wgpu::TextureView>,
	/// The number of mip levels of the picture.
	pub mip_levels: u32,
	pub intensity: f32,
}

/// Gives the sky of the view (the `Environment` of its camera).
#[derive(Default)]
pub struct InputEnvironment;

impl RenderPass for InputEnvironment {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.input_environment", "Environment", PassCategory::Input)
			.output(OutputDesc::data("environment", data::ENVIRONMENT))
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		let key = ctx.view.key;
		let Some((sky, intensity)) = ctx
			.world
			.get_resource::<ExtractedViews>()
			.and_then(|views| views.0.iter().find(|view| view.key() == key))
			.and_then(|view| view.environment)
		else {
			return PassAction::Skip;
		};
		let texture = ctx.cache.texture(sky);
		let data = EnvironmentData {
			texture: texture.map(|texture| texture.view.clone()),
			mip_levels: texture.map_or(1, |texture| texture.texture.mip_level_count()),
			intensity,
		};
		ctx.set_data("environment", data);
		PassAction::Run
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}

/// The data of `zenith.input_sprites`: the sprites of the view.
#[derive(Clone, Debug)]
pub struct SpritesData {
	pub list: SpriteList,
	/// The buffer the batches count their instances in. `None` when nothing
	/// is drawn.
	pub instances: Option<wgpu::Buffer>,
}

/// Gives the sprites the view draws.
#[derive(Default)]
pub struct InputSprites;

impl RenderPass for InputSprites {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.input_sprites", "Sprites", PassCategory::Input)
			.output(OutputDesc::data("sprites", data::SPRITES))
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		let Some(lists) = ctx.world.get_resource::<SpriteLists>() else {
			return PassAction::Skip;
		};
		let Some(list) = lists.get(ctx.view.key) else {
			return PassAction::Skip;
		};
		let data = SpritesData {
			list: list.clone(),
			instances: lists.instance_buffer().cloned(),
		};
		ctx.set_data("sprites", data);
		PassAction::Run
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}
