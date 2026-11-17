//! `zenith.output`: ends the graph. What it reads is what the camera shows;
//! the passes it does not depend on are left out.

use zenith_error::ZResult;

use crate::graph::{ExecContext, InputDesc, PassCategory, PassDesc, RenderPass, SlotKind};

#[derive(Default)]
pub struct Output;

impl RenderPass for Output {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.output", "Output", PassCategory::Output).input(InputDesc::read("color", SlotKind::Color))
	}

	fn exec(&self, _: &mut ExecContext<'_>) -> ZResult<()> {
		Ok(())
	}
}
