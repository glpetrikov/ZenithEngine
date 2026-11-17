//! `zenith.opaque`: draws the opaque and masked meshes into an HDR texture
//! with a depth buffer.
//!
//! It is lit by the lights, the environment and the shadow map it is given
//! (see `standard.wgsl`). A mesh or a material that is not loaded yet is not
//! drawn.

use zenith_error::ZResult;

use super::scene::MeshDraw;
use crate::graph::{
	ExecContext, InitContext, InputDesc, OutputDesc, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc,
	PrepareContext, RenderPass, SlotKind, TextureDesc, pass::data,
};

const DEFAULT_CLEAR: [f32; 4] = [0.1, 0.2, 0.5, 1.0];

pub struct Opaque {
	clear: [f32; 4],
	draw: MeshDraw,
}

impl Default for Opaque {
	fn default() -> Self {
		Self {
			clear: DEFAULT_CLEAR,
			draw: MeshDraw::new(false),
		}
	}
}

impl RenderPass for Opaque {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.opaque", "Opaque", PassCategory::Geometry)
			.input(InputDesc::read("camera", SlotKind::data(data::CAMERA)))
			.input(InputDesc::read("meshes", SlotKind::data(data::MESHES)))
			.input(InputDesc::read("lights", SlotKind::data(data::LIGHTS)).optional())
			.input(InputDesc::read("environment", SlotKind::data(data::ENVIRONMENT)).optional())
			.input(InputDesc::read("shadow_map", SlotKind::Depth).optional())
			.output(OutputDesc::create("color", SlotKind::Color, TextureDesc::HDR))
			.output(OutputDesc::create("depth", SlotKind::Depth, TextureDesc::DEPTH))
			.param(ParamDesc::color("clear_color", DEFAULT_CLEAR).tooltip("What the scene is cleared to"))
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.clear = params.vec4("clear_color").unwrap_or(DEFAULT_CLEAR);
		Ok(())
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.draw.init(ctx)
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.draw.prepare(ctx)
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		self.draw.exec(ctx, "opaque", self.clear)
	}
}
