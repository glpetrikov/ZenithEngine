//! The event loop: the window, its events, and when the editor ends.

use std::{
	sync::atomic::{AtomicBool, Ordering},
	time::{Duration, Instant},
};

use winit::{
	application::ApplicationHandler,
	event::{DeviceEvent, DeviceId, ElementState, KeyEvent, WindowEvent},
	event_loop::{ActiveEventLoop, ControlFlow},
	keyboard::{Key, NamedKey},
	window::WindowId,
};
use zenith_error::ZInternalResult;

use crate::{editor::Editor, gfx::Gfx, panels, recent};

/// Set by Ctrl+C, SIGTERM or closing the console.
pub static QUIT: AtomicBool = AtomicBool::new(false);

/// How often a window that cannot be seen (minimized) looks at the world.
const HIDDEN_POLL: Duration = Duration::from_millis(50);

pub struct EditorApp {
	gfx: Option<Gfx>,
	editor: Editor,
	last_frame: Instant,
	ctrl: bool,
	/// Why the editor could not start, to be returned from `main`.
	pub failure: Option<zenith_error::Report>,
	_log_guard: zenith_log::LogGuard,
}

impl EditorApp {
	pub fn new(log_guard: zenith_log::LogGuard) -> Self {
		Self {
			gfx: None,
			editor: Editor::new(recent::default_file()),
			last_frame: Instant::now(),
			ctrl: false,
			failure: None,
			_log_guard: log_guard,
		}
	}

	fn redraw(&mut self) {
		let Self {
			gfx,
			editor,
			last_frame,
			..
		} = self;
		let Some(gfx) = gfx else { return };
		let now = Instant::now();
		let dt = now.duration_since(*last_frame).as_secs_f32();
		*last_frame = now;
		if let Err(e) = gfx.frame(|ui| panels::draw(editor, ui)) {
			zenith_log::error!("The frame failed: {e}");
			self.failure = Some(e);
			editor.exit = true;
		}
		editor.end_frame(dt);
	}

	fn key(&mut self, event: &KeyEvent) {
		if event.state != ElementState::Pressed || event.repeat {
			return;
		}
		let wants_text = self.gfx.as_ref().is_some_and(Gfx::wants_text_input);
		match &event.logical_key {
			Key::Character(text) if self.ctrl && text.eq_ignore_ascii_case("s") => self.editor.save_all(),
			Key::Named(NamedKey::Escape)
				if !self.editor.viewport_focused && !self.editor.dialog_open && !wants_text =>
			{
				self.editor.request_exit();
			}
			_ => {}
		}
	}
}

impl ApplicationHandler for EditorApp {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.gfx.is_some() {
			return;
		}
		let layout = dirs::data_local_dir().map(|dir| dir.join("ZenithEngine").join("EditorLayout.ini"));
		match Gfx::new(event_loop, layout) {
			Ok(gfx) => self.gfx = Some(gfx),
			Err(e) => {
				zenith_log::error!("The editor cannot start: {e}");
				self.failure = Some(e);
				event_loop.exit();
			}
		}
	}

	fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
		let Some(gfx) = &mut self.gfx else { return };
		gfx.handle_event(&event);
		self.editor.window_event(&event);
		match event {
			WindowEvent::CloseRequested => self.editor.request_exit(),
			WindowEvent::Resized(size) => gfx.resize(size),
			WindowEvent::ModifiersChanged(modifiers) => self.ctrl = modifiers.state().control_key(),
			WindowEvent::KeyboardInput { event, .. } => self.key(&event),
			WindowEvent::RedrawRequested => self.redraw(),
			_ => {}
		}
	}

	fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
		self.editor.device_event(&event);
	}

	fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
		if QUIT.swap(false, Ordering::SeqCst) {
			zenith_log::info!("Stopped from outside: the unsaved changes go to the backup");
			self.editor.backup_and_exit();
		}
		if self.editor.exit {
			event_loop.exit();
			return;
		}
		let Some(gfx) = &self.gfx else { return };
		if gfx.window.is_minimized() == Some(true) {
			event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + HIDDEN_POLL));
		} else {
			event_loop.set_control_flow(ControlFlow::Poll);
			gfx.window.request_redraw();
		}
	}

	/// Every way out comes here: the window closed, Escape, Ctrl+C. The UI and
	/// the GPU are let go of in order while the window still exists.
	fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
		self.gfx = None;
		zenith_log::info!("The editor is closed");
	}
}

/// Runs the editor until it ends.
pub fn run(log_guard: zenith_log::LogGuard) -> ZInternalResult<()> {
	// TODO: add recent projects choose(but if not with --path launch flag).
	let event_loop =
		winit::event_loop::EventLoop::new().map_err(|e| zenith_error::eyre!("cannot start the event loop: {e}"))?;
	let mut app = EditorApp::new(log_guard);
	event_loop
		.run_app(&mut app)
		.map_err(|e| zenith_error::eyre!("the event loop failed: {e}"))?;
	app.failure.take().map_or(Ok(()), Err)
}
