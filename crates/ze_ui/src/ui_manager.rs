use std::{cell::RefCell, rc::Rc};

use yakui_wgpu::SurfaceInfo;

pub struct UiManager {
	pub yak: yakui::Yakui,
	pub winit: yakui_winit::YakuiWinit,
	pub wgpu: yakui_wgpu::YakuiWgpu,
	pub buffers: yakui_wgpu::Buffers,
	// `yakui_wgpu::YakuiWgpu` holds its own device/queue internally for
	// painting, but doesn't expose them back out. `UIImage` needs a
	// `wgpu::Device`/`Queue` to decode an asset into a `wgpu::Texture` before
	// handing the resulting view to `wgpu.add_texture`, so keep our own
	// clones here (cheap -- both types are thin Arc-backed handles).
	pub device: wgpu::Device,
	pub queue: wgpu::Queue,
	// The windowing system can deliver the first RedrawRequested before
	// UISystem has ever ticked (e.g. an implicit initial redraw on window
	// creation, which fires before the app's first about_to_wait-driven
	// system update). Painting `yak` before it has had one start()/finish()
	// layout pass panics inside yakui's paint_dom on a missing layout entry,
	// so paint() must no-op until UISystem has run at least once.
	has_laid_out: bool,
}

impl UiManager {
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, window: &winit::window::Window) -> Self {
		let mut yak = yakui::Yakui::new();
		let mut winit = yakui_winit::YakuiWinit::new(window);

		// Pin yakui's layout to a 1.0 scale factor and stop yakui_winit from
		// ever changing it. yakui lays widgets out against
		// `unscaled_viewport.size() / scale_factor` (its "logical" space), but
		// everything on our side -- `ViewportInfo`, the renderer surface, the
		// screen positions/sizes `UISystem` hands to `yakui::offset`/
		// `constrained`, and the reference-resolution canvas scaler -- works in
		// physical pixels. yakui_winit defaults to `auto_scale`, driving that
		// scale factor from `window.scale_factor()`; on any display whose scale
		// factor isn't 1.0 (HiDPI, fractional scaling) that makes yakui's
		// logical space `scale_factor`x smaller than the physical space we
		// compute positions in, so every anchored element lands
		// `scale_factor`x too far from its anchor -- edge/corner elements end up
		// at the wrong margin, and the error grows with viewport size. Forcing
		// scale factor 1.0 makes yakui's layout space exactly our physical-pixel
		// space; resolution independence is handled explicitly by the canvas
		// scaler, so we don't want the OS scale factor applied on top. This must
		// happen before the init-consuming event below, otherwise that event
		// (while `auto_scale` is still on) would set the scale factor from the
		// window first.
		winit.set_automatic_scale_factor(false);
		yak.set_scale_factor(1.0);

		// yakui_winit sizes itself to the OS window on its first
		// `handle_window_event` call. In the editor, yakui is painted into
		// the viewport panel's own texture (a sub-region of the window, kept
		// in sync via `set_viewport_size`), not the full window. Consume
		// that one-time "init" size snapshot now, with a harmless event,
		// before it can later overwrite the correct viewport-texture size
		// with the wrong (full window) one on the first real pointer event.
		winit.handle_window_event(&mut yak, &winit::event::WindowEvent::Focused(true));

		// One-shot confirmation that the scale factor is actually pinned after
		// construction (and that the init event above didn't put it back). If
		// this ever prints something other than 1.0, the anchor math (which
		// assumes physical pixels) and yakui's paint (which divides by
		// `scale_factor`) are already out of sync.
		ze_log::debug!(
			"[ui] UiManager::new: yakui scale_factor = {} (expected 1.0), surface_size = {:?}",
			yak.scale_factor(),
			yak.surface_size()
		);

		let wgpu = yakui_wgpu::YakuiWgpu::new(device.clone(), queue.clone());
		let buffers = wgpu.buffers();
		Self {
			yak,
			winit,
			wgpu,
			buffers,
			device: device.clone(),
			queue: queue.clone(),
			has_laid_out: false,
		}
	}

	/// Marks that `yak` has completed at least one `start()`/`finish()` layout
	/// pass, so `paint()` is now safe to call. Called by `UISystem` after
	/// `yak.finish()`.
	pub const fn mark_laid_out(&mut self) { self.has_laid_out = true; }

	pub fn handle_window_event(&mut self, event: &winit::event::WindowEvent) {
		self.winit.handle_window_event(&mut self.yak, event);
	}

	/// Syncs yakui's surface size and viewport to the actual render target
	/// the UI is painted into (the editor's viewport-panel texture), which is
	/// generally smaller than and decoupled from the OS window `winit`
	/// tracks. Without this, yakui's layout/paint keep their `Vec2::ONE`
	/// defaults and every widget renders as an oversized quad that fills the
	/// whole render target.
	pub fn set_viewport_size(&mut self, width: f32, height: f32) { self.sync_layout_viewport(width, height); }

	/// Force yakui to lay out and paint in exactly `width`x`height` pixels with
	/// a 1.0 scale factor and a zero-origin viewport -- i.e. the identity
	/// coordinate space.
	///
	/// This is the single authority for yakui's layout/paint frame of
	/// reference. `UISystem` calls it every frame with the exact
	/// `ViewportInfo` size it also computes anchor positions and the canvas
	/// scale from, so the paint-time normalization yakui applies
	/// (`(pos * scale_factor + unscaled_viewport.pos()) / surface_size`, see
	/// yakui-core `PaintDom`) is guaranteed to invert our anchor math rather
	/// than drift away from it.
	///
	/// Driving this from `UISystem` every frame -- instead of relying on the
	/// renderer's resize path (which early-returns when the size is unchanged)
	/// or on `yakui_winit` reacting to window events (which tracks the OS
	/// window, applies the OS scale factor, and fires on a different schedule)
	/// -- is what stops edge/corner-anchored UI from landing at the wrong,
	/// size-dependent margin.
	pub fn sync_layout_viewport(&mut self, width: f32, height: f32) {
		let size = yakui::Vec2::new(width.max(1.0), height.max(1.0));
		self.yak.set_scale_factor(1.0);
		self.yak.set_surface_size(size);
		self.yak
			.set_unscaled_viewport(yakui::Rect::from_pos_size(yakui::Vec2::ZERO, size));
	}

	/// yakui's current layout/paint frame of reference, for diagnostics: the
	/// scale factor, the surface size the paint stage normalizes against, and
	/// the unscaled viewport rect. `UISystem` logs this against the
	/// `ViewportInfo` it laid out with so a mismatch is visible in the log.
	pub fn layout_frame_debug(&self) -> (f32, yakui::Vec2, yakui::Rect) {
		(
			self.yak.scale_factor(),
			self.yak.surface_size(),
			self.yak.layout_dom().unscaled_viewport(),
		)
	}

	pub fn paint(
		&mut self,
		encoder: &mut wgpu::CommandEncoder,
		color_attachment: &wgpu::TextureView,
		format: wgpu::TextureFormat,
		sample_count: u32,
	) {
		if !self.has_laid_out {
			return;
		}

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
