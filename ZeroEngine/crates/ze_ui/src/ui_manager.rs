use std::{cell::RefCell, rc::Rc};

use yakui_wgpu::SurfaceInfo;

pub struct UiManager {
	pub yak: yakui::Yakui,
	pub winit: yakui_winit::YakuiWinit,
	pub wgpu: yakui_wgpu::YakuiWgpu,
	pub buffers: yakui_wgpu::Buffers,
}

impl UiManager {
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, window: &winit::window::Window) -> Self {
		let mut yak = yakui::Yakui::new();
		let mut winit = yakui_winit::YakuiWinit::new(window);

		// yakui_winit sizes itself to the OS window on its first
		// `handle_window_event` call. In the editor, yakui is painted into
		// the viewport panel's own texture (a sub-region of the window, kept
		// in sync via `set_viewport_size`), not the full window. Consume
		// that one-time "init" size snapshot now, with a harmless event,
		// before it can later overwrite the correct viewport-texture size
		// with the wrong (full window) one on the first real pointer event.
		winit.handle_window_event(&mut yak, &winit::event::WindowEvent::Focused(true));

		let wgpu = yakui_wgpu::YakuiWgpu::new(device.clone(), queue.clone());
		let buffers = wgpu.buffers();
		Self {
			yak,
			winit,
			wgpu,
			buffers,
		}
	}

	pub fn handle_window_event(&mut self, event: &winit::event::WindowEvent) {
		self.winit.handle_window_event(&mut self.yak, event);
	}

	/// Syncs yakui's surface size and viewport to the actual render target
	/// the UI is painted into (the editor's viewport-panel texture), which is
	/// generally smaller than and decoupled from the OS window `winit`
	/// tracks. Without this, yakui's layout/paint keep their `Vec2::ONE`
	/// defaults and every widget renders as an oversized quad that fills the
	/// whole render target.
	pub fn set_viewport_size(&mut self, width: f32, height: f32) {
		let size = yakui::Vec2::new(width, height);
		self.yak.set_surface_size(size);
		self.yak
			.set_unscaled_viewport(yakui::Rect::from_pos_size(yakui::Vec2::ZERO, size));
	}

	pub fn paint(
		&mut self,
		encoder: &mut wgpu::CommandEncoder,
		color_attachment: &wgpu::TextureView,
		format: wgpu::TextureFormat,
		sample_count: u32,
	) {
		let surface_info = SurfaceInfo {
			format,
			sample_count,
			color_attachment,
			resolve_target: None,
		};
		self.wgpu
			.paint_with_encoder(&mut self.yak, &mut self.buffers, encoder, surface_info);
	}
}

#[derive(Clone)]
pub struct UiManagerHandle(Rc<RefCell<UiManager>>);

impl UiManagerHandle {
	pub fn new(manager: UiManager) -> Self { Self(Rc::new(RefCell::new(manager))) }

	pub fn borrow_mut(&self) -> std::cell::RefMut<'_, UiManager> { self.0.borrow_mut() }
}
