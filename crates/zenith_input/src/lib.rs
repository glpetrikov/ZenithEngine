//! Keyboard, mouse, gamepad and actions, as a value that the game asks.
//!
//! [`Input`] stores the state of every device. Events from anywhere reach it as
//! [`InputEvent`]s: from the window with the `winit` feature, from real
//! gamepads with the `gamepad` feature, from a test or an emulated gamepad as
//! plain values. With the `ecs` feature (default) it is a resource of the
//! world. Nothing here opens a window or a device: what the game asks of them
//! (vibrate a pad, hide the cursor) is recorded for the code that owns them.
//!
//! ```
//! use zenith_input::{Input, InputEvent, ZGamePad, ZKeyCode};
//!
//! let mut input = Input::default();
//! input.apply(InputEvent::KeyDown(ZKeyCode::Space));
//! assert!(input.is_key_pressed(ZKeyCode::Space));
//! assert!(!input.is_gamepad_pressed(ZGamePad::South, 0));
//! ```

// rustfmt (format_code_in_doc_comments) indents the examples with tabs, like all code here.
#![allow(clippy::tabs_in_doc_comments)]

mod action;
mod devices;
mod event;
#[cfg(feature = "gamepad")]
mod gamepads;
mod input;
mod key;
mod names;
mod output;
mod record;
mod settings;
#[cfg(feature = "winit")]
mod window;

pub use action::{Binding, Source, VectorAction};
pub use devices::{GamepadInfo, ZGamePad, ZGamePadAxis, ZGamepadKind, ZMouseButton, ZPowerInfo, ZStick};
pub use event::InputEvent;
#[cfg(feature = "gamepad")]
pub use gamepads::Gamepads;
pub use glam::Vec2;
pub use input::{GamepadMut, GamepadRef, Input};
pub use key::ZKeyCode;
pub use output::{CursorMode, PadCommand, RumbleEffect};
pub use record::{RecordedFrame, Recording, Replay};
pub use settings::{GamepadSettings, InputFile, InputSettings, MouseSettings};
#[cfg(feature = "winit")]
pub use window::{apply_cursor_mode, events_from_device_event, events_from_window_event};
