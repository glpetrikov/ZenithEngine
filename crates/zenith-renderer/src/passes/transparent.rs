//! `zenith.transparent`: the meshes that blend, from far to near, over the
//! color the passes before drew. They are tested against the depth and do not
//! write it.

use zenith_error::ZResult;

use super::scene::MeshDraw;
use crate::graph::{
	ExecContext, InitContext, InputDesc, OutputDesc, PassAction, PassCategory, PassDesc, PrepareContext, RenderPass,
	SlotKind, pass::data,
};

pub struct Transparent {
	draw: MeshDraw,
}

impl Default for Transparent {
	fn default() -> Self {
		Self {
			draw: MeshDraw::new(true),
		}
	}
}

impl RenderPass for Transparent {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.transparent", "Transparent", PassCategory::Geometry)
			.input(InputDesc::read("camera", SlotKind::data(data::CAMERA)))
			.input(InputDesc::read("meshes", SlotKind::data(data::MESHES)))
			.input(InputDesc::read("lights", SlotKind::data(data::LIGHTS)).optional())
			.input(InputDesc::read("environment", SlotKind::data(data::ENVIRONMENT)).optional())
			.input(InputDesc::read("shadow_map", SlotKind::Depth).optional())
			.input(InputDesc::write("color", SlotKind::Color))
			.input(InputDesc::read("depth", SlotKind::Depth))
			.output(OutputDesc::pass_through("color", SlotKind::Color, "color"))
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.draw.init(ctx)
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.draw.prepare(ctx)
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		self.draw.exec(ctx, "transparent", [0.0; 4])
	}
}
