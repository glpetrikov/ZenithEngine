//! The input nodes: what a view is drawn from, given to the passes through
//! data slots.
//!
//! `zenith.input_camera` and `zenith.input_meshes` for now; the lights, the
//! environment and the sprites come with the passes that need them.
//!
//! The input nodes are the only nodes of the engine that read the world.

use zenith_error::ZResult;

use crate::{
	graph::{
		ExecContext, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc, PrepareContext, RenderPass,
		pass::{OutputDesc, data},
	},
	prepare::{CameraUniform, DrawLists, MeshList, PreparedViews},
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
