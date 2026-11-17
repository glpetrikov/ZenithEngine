//! The passes of the engine.
//!
//! [`register`] puts them in a [`PassRegistry`]; `zenith-renderer` does it
//! for its own registry at start.

pub mod editor;
pub mod inputs;
pub mod opaque;
pub mod output;
pub mod scene;
pub mod shadow;
pub mod sky;
pub mod sprites;
pub mod tonemap;
pub mod transparent;

use crate::graph::PassRegistry;

/// Registers the passes of the engine.
pub fn register(passes: &mut PassRegistry) {
	passes.register::<inputs::InputCamera>();
	passes.register::<inputs::InputMeshes>();
	passes.register::<inputs::InputLights>();
	passes.register::<inputs::InputEnvironment>();
	passes.register::<shadow::Shadow>();
	passes.register::<sky::Sky>();
	passes.register::<transparent::Transparent>();
	passes.register::<inputs::InputSprites>();
	passes.register::<sprites::Sprites>();
	passes.register::<opaque::Opaque>();
	passes.register::<tonemap::Tonemap>();
	passes.register::<output::Output>();
	passes.register::<editor::Grid>();
	passes.register::<editor::Gizmos>();
	passes.register::<editor::Outline>();
}
