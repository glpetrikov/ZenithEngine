use std::any::Any;

pub trait System {
	fn name(&self) -> &'static str;

	fn update(&mut self, scene: &mut crate::Scene, dt: f32) -> ze_core::Result<()>;

	fn as_any_mut(&mut self) -> &mut dyn Any;
}
