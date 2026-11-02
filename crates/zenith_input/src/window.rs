//! Input from `winit` windows.

use glam::Vec2;
use winit::{
	event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
	keyboard::PhysicalKey,
	window::{CursorGrabMode, Window},
};
use zenith_error::{ZResult, ZenithError};

use crate::{CursorMode, Input, InputEvent, ZKeyCode, ZMouseButton};

/// How many pixels of a touchpad scroll count as one line of a wheel.
const PIXELS_PER_LINE: f32 = 40.0;

impl From<MouseButton> for ZMouseButton {
	fn from(button: MouseButton) -> Self {
		match button {
			MouseButton::Left => Self::Left,
			MouseButton::Right => Self::Right,
			MouseButton::Middle => Self::Middle,
			MouseButton::Back => Self::Back,
			MouseButton::Forward => Self::Forward,
			MouseButton::Other(number) => Self::Other(number),
		}
	}
}

/// The input events a window event stands for (none for most of them).
#[must_use]
pub fn events_from_window_event(event: &WindowEvent) -> Vec<InputEvent> {
	match event {
		WindowEvent::KeyboardInput { event, .. } => {
			let mut events = Vec::new();
			// The repeat of a held key is not a new press, but it types again.
			if !event.repeat
				&& let PhysicalKey::Code(code) = event.physical_key
				&& let Some(key) = ZKeyCode::from_winit(code)
			{
				events.push(match event.state {
					ElementState::Pressed => InputEvent::KeyDown(key),
					ElementState::Released => InputEvent::KeyUp(key),
				});
			}
			if event.state == ElementState::Pressed
				&& let Some(text) = &event.text
			{
				events.push(InputEvent::Text(text.to_string()));
			}
			events
		}
		WindowEvent::MouseInput { state, button, .. } => {
			let button = ZMouseButton::from(*button);
			vec![match state {
				ElementState::Pressed => InputEvent::MouseDown(button),
				ElementState::Released => InputEvent::MouseUp(button),
			}]
		}
		WindowEvent::CursorMoved { position, .. } => {
			vec![InputEvent::MouseMoved(Vec2::new(position.x as f32, position.y as f32))]
		}
		WindowEvent::CursorLeft { .. } => vec![InputEvent::MouseLeft],
		WindowEvent::MouseWheel { delta, .. } => {
			let lines = match delta {
				MouseScrollDelta::LineDelta(x, y) => Vec2::new(*x, *y),
				MouseScrollDelta::PixelDelta(position) => {
					Vec2::new(position.x as f32, position.y as f32) / PIXELS_PER_LINE
				}
			};
			vec![InputEvent::Scroll(lines)]
		}
		WindowEvent::Focused(focused) => vec![InputEvent::Focus(*focused)],
		WindowEvent::Ime(winit::event::Ime::Commit(text)) => vec![InputEvent::Text(text.clone())],
		_ => Vec::new(),
	}
}

/// The input events a device event stands for.
#[must_use]
pub fn events_from_device_event(event: &DeviceEvent) -> Vec<InputEvent> {
	match event {
		DeviceEvent::MouseMotion { delta: (dx, dy) } => {
			vec![InputEvent::MouseMotion(Vec2::new(*dx as f32, *dy as f32))]
		}
		_ => Vec::new(),
	}
}

/// Carries out a cursor request on a window: the cursor is shown, hidden, or
/// hidden and kept inside the window. Call it with what
/// [`Input::take_cursor_mode`] gives.
///
/// # Errors
/// `Unsupported` if the system refuses to grab the cursor (in a browser the
/// player has to click on the page first). The cursor is hidden anyway.
pub fn apply_cursor_mode(window: &Window, mode: CursorMode) -> ZResult<()> {
	window.set_cursor_visible(mode == CursorMode::Normal);
	let grab = match mode {
		CursorMode::Normal | CursorMode::Hidden => CursorGrabMode::None,
		CursorMode::Locked => CursorGrabMode::Locked,
	};
	// Some systems can only keep the cursor inside the window, not lock it.
	window
		.set_cursor_grab(grab)
		.or_else(|error| {
			if grab == CursorGrabMode::Locked {
				window.set_cursor_grab(CursorGrabMode::Confined)
			} else {
				Err(error)
			}
		})
		.map_err(|e| ZenithError::Unsupported(format!("the cursor cannot be grabbed: {e}")))
}

impl Input {
	/// Takes a window event into account (the keys, the mouse, the wheel, the
	/// typed text, the focus). Call it for every event the window gets.
	pub fn handle_window_event(&mut self, event: &WindowEvent) {
		for event in events_from_window_event(event) {
			self.apply(event);
		}
	}

	/// Takes a device event into account (the raw movement of the mouse, which
	/// goes on when the cursor is grabbed and stops at the edge of the screen).
	pub fn handle_device_event(&mut self, event: &DeviceEvent) {
		for event in events_from_device_event(event) {
			self.apply(event);
		}
	}
}

#[cfg(test)]
mod tests {
	use winit::keyboard::KeyCode;

	use super::*;

	#[test]
	fn every_key_maps_both_ways() {
		for key in ZKeyCode::ALL {
			assert_eq!(
				ZKeyCode::from_winit(key.to_winit()),
				Some(*key),
				"{key} goes to winit and back"
			);
		}
		assert_eq!(ZKeyCode::from_winit(KeyCode::KeyW), Some(ZKeyCode::W), "KeyW");
		assert_eq!(
			ZKeyCode::from_winit(KeyCode::ShiftLeft),
			Some(ZKeyCode::LeftShift),
			"ShiftLeft"
		);
		assert_eq!(ZKeyCode::from_winit(KeyCode::ArrowUp), Some(ZKeyCode::Up), "ArrowUp");
	}

	#[test]
	fn mouse_buttons_map() {
		assert_eq!(ZMouseButton::from(MouseButton::Left), ZMouseButton::Left, "left");
		assert_eq!(
			ZMouseButton::from(MouseButton::Other(9)),
			ZMouseButton::Other(9),
			"other"
		);
	}

	#[test]
	fn the_focus_and_the_raw_motion_become_events() {
		assert_eq!(
			events_from_window_event(&WindowEvent::Focused(false)),
			vec![InputEvent::Focus(false)],
			"focus"
		);
		assert_eq!(
			events_from_device_event(&DeviceEvent::MouseMotion { delta: (1.0, -2.0) }),
			vec![InputEvent::MouseMotion(Vec2::new(1.0, -2.0))],
			"raw motion"
		);
		assert!(
			events_from_window_event(&WindowEvent::RedrawRequested).is_empty(),
			"other events mean nothing"
		);
	}
}
