pub mod components;
pub mod system;
pub mod ui_manager;

pub use components::*;
pub use system::*;
pub use ui_manager::*;
use ze_ecs::ComponentRegistry;

pub fn register_ui_components(registry: &mut ComponentRegistry) {
	registry.register_with_display_name::<UIButton>("ze.ui.button", "UI Button");
	registry.register_with_display_name::<UIBar>("ze.ui.bar", "UI Bar");
	registry.register_with_display_name::<UIText>("ze.ui.text", "UI Text");
}
