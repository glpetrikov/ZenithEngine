use std::{cell::Cell, collections::HashMap};

use yakui::{Constraints, Rect, Vec2, Vec4};
use ze_assets::ResourceManager;
use ze_core::Result;
use ze_ecs::{
	ActiveCameraView, EntityId, Scene, System, UiReferenceResolution, ViewportInfo,
	shipyard::{IntoIter, UniqueView, View},
};

use crate::{
	components::{UIAnchorMode, UIBar, UIButton, UIImage, UIRect, UIText},
	image_texture_cache::UiImageTextureCache,
	image_widget::TexturedQuad,
	ui_manager::{UiManager, UiManagerHandle},
};

/// Font size used for the optional label rendered on top of a `UIBar`.
const BAR_LABEL_FONT_SIZE: f32 = 14.0;

struct ButtonSnapshot {
	entity: EntityId,
	screen_pos: Vec2,
	size: Vec2,
	text: String,
	// Captured for parity with the component's data, but yakui's stock
	// Button widget has no font-size override hook (pre-existing gap, not
	// introduced by z-ordering/anchor changes).
	#[allow(dead_code)]
	font_size: f32,
	color: [f32; 4],
	hover_color: [f32; 4],
	pressed_color: [f32; 4],
	z_index: i32,
}

struct BarSnapshot {
	entity: EntityId,
	screen_pos: Vec2,
	size: Vec2,
	current: f32,
	max: f32,
	color: [f32; 4],
	bg_color: [f32; 4],
	text: Option<String>,
	/// Font size for the optional centered label, already multiplied by the
	/// bar's canvas scale so the label stays proportional to the scaled bar.
	label_font_size: f32,
	z_index: i32,
}

struct TextSnapshot {
	entity: EntityId,
	screen_pos: Vec2,
	size: Vec2,
	text: String,
	font_size: f32,
	color: [f32; 4],
	z_index: i32,
}

struct ImageSnapshot {
	entity: EntityId,
	screen_pos: Vec2,
	size: Vec2,
	// Resolved once per frame in Pass 1 (needs `&mut UiImageTextureCache` and
	// the yakui/wgpu handles) -- `None` for an unassigned or failed-to-load
	// texture, in which case Pass 2 draws nothing for this entity.
	texture: Option<(yakui::TextureId, [f32; 4])>,
	color: [f32; 4],
	z_index: i32,
}

/// One paintable UI element, tagged with the data needed to order it against
/// every other element regardless of its component type.
enum UiElement {
	Button(ButtonSnapshot),
	Bar(BarSnapshot),
	Text(TextSnapshot),
	Image(ImageSnapshot),
}

impl UiElement {
	const fn sort_key(&self) -> (i32, EntityId) {
		match self {
			Self::Button(snap) => (snap.z_index, snap.entity),
			Self::Bar(snap) => (snap.z_index, snap.entity),
			Self::Text(snap) => (snap.z_index, snap.entity),
			Self::Image(snap) => (snap.z_index, snap.entity),
		}
	}
}

/// The screen-space point a `UIRect`'s `x`/`y` offset is measured from, and
/// the point of its own `width`/`height` box that gets aligned there, both
/// derived from `rect.anchor_point`'s fraction of `viewport_size`/the rect's
/// own size. For `UIAnchorPoint::TopLeft` (the default) both fractions are
/// `0.0`, so this collapses to `(0.0, 0.0)` regardless of `viewport_size` --
/// exactly reproducing the old fixed-pixel-from-top-left behavior.
///
/// `scale` is the canvas scale factor (see `UISystem::update`); it multiplies
/// the element's own box size before the pivot fraction is taken, so a scaled
/// element still aligns its (scaled) corner/edge/center to the anchor. The
/// viewport anchor itself is not scaled -- it's already an absolute position on
/// the live viewport.
fn anchor_and_pivot(rect: &UIRect, viewport_size: Vec2, scale: f32) -> (Vec2, Vec2) {
	let (fx, fy) = rect.anchor_point.fractions();
	let anchor = Vec2::new(viewport_size.x * fx, viewport_size.y * fy);
	let pivot = Vec2::new(rect.width.max(0.0) * scale * fx, rect.height.max(0.0) * scale * fy);
	(anchor, pivot)
}

/// Resolves the screen-space pixel position (top-left corner of the element's
/// box) for a UI rect.
///
/// In `UIAnchorMode::ScreenSpaceOverlay`, the anchor is `rect.anchor_point`'s
/// fraction of the current `viewport_size` (e.g. the bottom-right corner of
/// the screen for `UIAnchorPoint::BottomRight`), so the element stays
/// correctly positioned relative to that edge/corner/center as the window is
/// resized; camera projection never runs, so it's also unaffected by camera
/// movement. `rect.x/y` is a pixel offset from that anchor, and the same
/// `anchor_point` fraction of the element's own size is subtracted so e.g. a
/// `BottomRight`-anchored element's bottom-right corner (not its top-left)
/// sits at the anchor point.
///
/// In `UIAnchorMode::WorldSpace`, if the scene has an active camera and the
/// entity has a world transform, `rect.x/y` is treated as a pixel offset from
/// the entity's projected screen position (with the same pivot subtraction
/// applied). Otherwise the anchor falls back to `rect.anchor_point`'s fraction
/// of `viewport_size`, exactly as in `ScreenSpaceOverlay`.
///
/// `scale` is the canvas scale factor already resolved for this element's
/// anchor mode (`1.0` for `WorldSpace`; the match-by-width factor for
/// `ScreenSpaceOverlay`). It multiplies the authored `x`/`y` offset (and, via
/// `anchor_and_pivot`, the pivot) so a UI authored at the reference resolution
/// keeps the same relative layout as the viewport grows or shrinks.
fn resolve_screen_pos(
	scene: &Scene,
	camera: Option<&ActiveCameraView>,
	viewport_size: Vec2,
	entity: EntityId,
	rect: &UIRect,
	anchor_mode: UIAnchorMode,
	scale: f32,
) -> Vec2 {
	let (screen_anchor, pivot) = anchor_and_pivot(rect, viewport_size, scale);
	let offset = Vec2::new(rect.x * scale, rect.y * scale);

	if anchor_mode == UIAnchorMode::ScreenSpaceOverlay {
		return screen_anchor - pivot + offset;
	}

	let anchor = camera.and_then(|camera| {
		scene
			.world_transform(entity)
			.and_then(|transform| camera.project_to_screen(transform.position))
	});

	anchor.map_or(screen_anchor - pivot + offset, |anchor| {
		Vec2::new(anchor.x, anchor.y) - pivot + offset
	})
}

/// A negative width/height (e.g. from a mis-edited rect in the Inspector or
/// a hand-edited scene file) produces a negative `Constraints::tight` size.
/// yakui's layout isn't guaranteed to handle that gracefully, so clamp at
/// the source rather than let malformed data reach yakui at all.
fn ui_rect_size(rect: &UIRect, scale: f32) -> Vec2 {
	Vec2::new(rect.width.max(0.0) * scale, rect.height.max(0.0) * scale)
}

/// The canvas scale factor that applies to an element with the given anchor
/// mode. Only `ScreenSpaceOverlay` (HUD-style) UI is scaled by the
/// match-by-width canvas factor -- `WorldSpace` UI is pinned to a world object
/// via camera projection, so scaling it by window width would be wrong; it
/// always uses `1.0`, exactly reproducing its pre-scaler behavior.
const fn effective_scale(anchor_mode: UIAnchorMode, canvas_scale: f32) -> f32 {
	match anchor_mode {
		UIAnchorMode::ScreenSpaceOverlay => canvas_scale,
		UIAnchorMode::WorldSpace => 1.0,
	}
}

/// The match-by-width canvas scale factor for the current frame: the ratio of
/// the live viewport width to the reference (design) width. Computed fresh
/// every frame from the current `ViewportInfo`, never captured once, so it
/// tracks live window resizes in both directions. Falls back to `1.0` when
/// either width is non-positive (e.g. before the first real viewport exists),
/// so UI never collapses to zero size; `1.0` also means "viewport == reference"
/// leaves authored coordinates untouched.
fn canvas_scale(reference_width: f32, viewport_width: f32) -> f32 {
	if reference_width > 0.0 && viewport_width > 0.0 {
		viewport_width / reference_width
	} else {
		1.0
	}
}

pub struct UISystem {
	ui_manager: Option<UiManagerHandle>,
	// yakui's RenderTextWidget only re-shapes its cosmic-text buffer when the
	// text content (or font attrs) changes between frames, not when font_size
	// alone changes (see yakui-widgets RenderTextWidget::update/layout). Since
	// our Text widgets are re-issued every frame at a stable call site with
	// unchanged content, a SetFontSize() call would otherwise be silently
	// ignored. We track the last font_size we rendered per entity and, when
	// it changes, append a zero-width space to force the text-changed check
	// to trip and re-shape with the new metrics.
	text_font_size_cache: HashMap<EntityId, f32>,
	resources: ResourceManager,
	image_texture_cache: UiImageTextureCache,
	/// Last `(viewport_size, canvas_scale)` we emitted a layout-frame debug log
	/// for, so the per-frame diagnostic only fires when the viewport actually
	/// changes (e.g. a live resize) instead of every frame.
	last_logged_layout: Option<(Vec2, f32)>,
}

impl UISystem {
	pub fn new(handle: UiManagerHandle, resources: ResourceManager) -> Self {
		Self {
			ui_manager: Some(handle),
			text_font_size_cache: HashMap::new(),
			resources,
			image_texture_cache: UiImageTextureCache::new(),
			last_logged_layout: None,
		}
	}
}

impl System for UISystem {
	fn name(&self) -> &'static str { "UISystem" }

	#[allow(clippy::too_many_lines)]
	fn update(&mut self, scene: &mut Scene, _dt: f32) -> Result<()> {
		let Some(ref handle) = self.ui_manager else {
			return Ok(());
		};

		let mut manager = handle.borrow_mut();
		// A single `&mut UiManager` reborrow (rather than repeatedly deref'ing
		// the `RefMut` below) so the borrow checker can see `manager.device`,
		// `manager.queue`, and `manager.wgpu` as disjoint field borrows when
		// resolving `UIImage` textures.
		let manager: &mut UiManager = &mut manager;

		let active_camera: Option<ActiveCameraView> = scene
			.world()
			.borrow::<UniqueView<ActiveCameraView>>()
			.ok()
			.map(|view| *view);
		let viewport_size: Vec2 = scene
			.world()
			.borrow::<UniqueView<ViewportInfo>>()
			.map_or(Vec2::ZERO, |info| Vec2::new(info.size.x, info.size.y));

		// Match-by-width canvas scale: how big everything should be relative to
		// the design/reference resolution. `1.0` when the live viewport width
		// equals the reference width (so a scene authored at that resolution is
		// untouched), >1 on a wider window, <1 on a narrower one. Falls back to
		// the 1920x1080 default when the project didn't supply a
		// `UiReferenceResolution` unique. Applied per element via
		// `effective_scale` so only `ScreenSpaceOverlay` UI is affected.
		let reference_width = scene.world().borrow::<UniqueView<UiReferenceResolution>>().map_or_else(
			|_| UiReferenceResolution::default().size.x,
			|reference| reference.size.x,
		);
		let canvas_scale = canvas_scale(reference_width, viewport_size.x);

		// Pass 1: snapshot component data
		let button_snapshots: Vec<ButtonSnapshot> = {
			let world = scene.world();
			world.borrow::<View<UIButton>>().map_or_else(
				|_| Vec::new(),
				|buttons| {
					buttons
						.iter()
						.with_id()
						.map(|(entity, b)| {
							let scale = effective_scale(b.anchor_mode, canvas_scale);
							ButtonSnapshot {
								entity,
								screen_pos: resolve_screen_pos(
									scene,
									active_camera.as_ref(),
									viewport_size,
									entity,
									&b.rect,
									b.anchor_mode,
									scale,
								),
								size: ui_rect_size(&b.rect, scale),
								text: b.text.clone(),
								font_size: b.font_size * scale,
								color: b.color,
								hover_color: b.hover_color,
								pressed_color: b.pressed_color,
								z_index: b.z_index,
							}
						})
						.collect()
				},
			)
		};

		let bar_snapshots: Vec<BarSnapshot> = {
			let world = scene.world();
			world.borrow::<View<UIBar>>().map_or_else(
				|_| Vec::new(),
				|bars| {
					bars.iter()
						.with_id()
						.map(|(entity, b)| {
							let scale = effective_scale(b.anchor_mode, canvas_scale);
							BarSnapshot {
								entity,
								screen_pos: resolve_screen_pos(
									scene,
									active_camera.as_ref(),
									viewport_size,
									entity,
									&b.rect,
									b.anchor_mode,
									scale,
								),
								size: ui_rect_size(&b.rect, scale),
								current: b.current,
								max: b.max,
								color: b.color,
								bg_color: b.bg_color,
								text: b.text.clone(),
								label_font_size: BAR_LABEL_FONT_SIZE * scale,
								z_index: b.z_index,
							}
						})
						.collect()
				},
			)
		};

		let text_snapshots: Vec<TextSnapshot> = {
			let world = scene.world();
			world.borrow::<View<UIText>>().map_or_else(
				|_| Vec::new(),
				|texts| {
					texts
						.iter()
						.with_id()
						.map(|(entity, t)| {
							let scale = effective_scale(t.anchor_mode, canvas_scale);
							TextSnapshot {
								entity,
								screen_pos: resolve_screen_pos(
									scene,
									active_camera.as_ref(),
									viewport_size,
									entity,
									&t.rect,
									t.anchor_mode,
									scale,
								),
								size: ui_rect_size(&t.rect, scale),
								text: t.text.clone(),
								// Scale the rendered font by the canvas factor so text
								// grows/shrinks with the rest of the UI, then clamp: a
								// non-positive font_size reaches cosmic-text as a negative
								// TextStyle::to_metrics() -> Metrics::line_height, which sends
								// Buffer::shape_until_scroll's line-layout loop into a hang
								// (confirmed via debugger: stuck in BufferLine::layout /
								// ShapeLine::layout_to_buffer). cosmic-text has no internal
								// guard against this, so clamp before it ever reaches yakui.
								font_size: (t.font_size * scale).max(1.0),
								color: t.color,
								z_index: t.z_index,
							}
						})
						.collect()
				},
			)
		};

		// Resolving a texture may register a new GPU texture with yakui
		// (`&mut manager.wgpu`), so this is a plain loop rather than an
		// iterator chain like the snapshots above, which only ever read from
		// `scene`.
		let mut image_snapshots: Vec<ImageSnapshot> = Vec::new();
		{
			let world = scene.world();
			if let Ok(images) = world.borrow::<View<UIImage>>() {
				for (entity, image) in images.iter().with_id() {
					let scale = effective_scale(image.anchor_mode, canvas_scale);
					let screen_pos = resolve_screen_pos(
						scene,
						active_camera.as_ref(),
						viewport_size,
						entity,
						&image.rect,
						image.anchor_mode,
						scale,
					);
					let texture = self.image_texture_cache.resolve(
						&image.texture,
						&self.resources,
						&manager.device,
						&manager.queue,
						&mut manager.wgpu,
					);
					image_snapshots.push(ImageSnapshot {
						entity,
						screen_pos,
						size: ui_rect_size(&image.rect, scale),
						texture,
						color: image.color,
						z_index: image.z_index,
					});
				}
			}
		}

		let mut elements: Vec<UiElement> = Vec::with_capacity(
			button_snapshots.len() + bar_snapshots.len() + text_snapshots.len() + image_snapshots.len(),
		);
		elements.extend(button_snapshots.into_iter().map(UiElement::Button));
		elements.extend(bar_snapshots.into_iter().map(UiElement::Bar));
		elements.extend(text_snapshots.into_iter().map(UiElement::Text));
		elements.extend(image_snapshots.into_iter().map(UiElement::Image));
		elements.sort_by_key(UiElement::sort_key);

		// Pin yakui's layout/paint frame of reference to the exact same
		// `viewport_size` this system computed anchor positions and
		// `canvas_scale` from, every frame, right before laying out. yakui
		// normalizes painted positions as
		// `(pos * scale_factor + unscaled_viewport.pos()) / surface_size`; unless
		// those params equal our `viewport_size` (scale 1.0, zero origin), a
		// `ScreenSpaceOverlay` element placed at, say, the bottom-right corner
		// paints at the wrong, size-dependent margin. Doing it here (not via the
		// renderer resize path or yakui_winit's window-event tracking) makes the
		// two provably consistent regardless of event timing or OS scale factor.
		manager.sync_layout_viewport(viewport_size.x, viewport_size.y);

		// Diagnostic (debug level, only when the viewport changes): print the
		// size we laid out against next to yakui's actual layout frame, so a
		// drift between the two is visible in the log rather than only on
		// screen. `layout_scale` must read back as 1.0 and `surface` as
		// `viewport_size`; if they don't, the fix above isn't taking effect.
		if self.last_logged_layout.map(|(v, _)| v) != Some(viewport_size) {
			let (layout_scale, surface, unscaled) = manager.layout_frame_debug();
			ze_log::debug!(
				"[ui] layout frame: viewport_size={viewport_size:?} canvas_scale={canvas_scale:.4} | \
				 yakui scale_factor={layout_scale} surface={surface:?} unscaled_viewport={unscaled:?}"
			);
			self.last_logged_layout = Some((viewport_size, canvas_scale));
		}

		// Pass 2: build yakui widget tree in z_index (then entity id) order
		manager.yak.start();

		let mut button_results: Vec<(EntityId, bool, bool)> = Vec::new();
		let mut text_entities: Vec<EntityId> = Vec::new();

		for element in &elements {
			match element {
				UiElement::Button(snap) => {
					let clicked = Cell::new(false);
					let hovering = Cell::new(false);
					yakui::offset(snap.screen_pos, || {
						yakui::constrained(Constraints::tight(snap.size), || {
							let mut btn = yakui::widgets::Button::styled(snap.text.clone());
							btn.style.fill = yakui::Color::from_linear(Vec4::from_array(snap.color));
							btn.hover_style.fill = yakui::Color::from_linear(Vec4::from_array(snap.hover_color));
							btn.down_style.fill = yakui::Color::from_linear(Vec4::from_array(snap.pressed_color));
							let response = btn.show();
							clicked.set(response.clicked);
							hovering.set(response.hovering);
						});
					});
					button_results.push((snap.entity, clicked.get(), hovering.get()));
				}
				UiElement::Bar(snap) => {
					let fill_ratio = (snap.current / snap.max).clamp(0.0, 1.0);
					let fill_width = snap.size.x * fill_ratio;

					let bg_color = yakui::Color::from_linear(Vec4::from_array(snap.bg_color));
					let fg_color = yakui::Color::from_linear(Vec4::from_array(snap.color));

					yakui::offset(snap.screen_pos, || {
						// Background: full rect
						yakui::constrained(Constraints::tight(snap.size), || {
							yakui::colored_box(bg_color, snap.size);
						});
						// Foreground: fill_ratio width
						yakui::constrained(Constraints::tight(Vec2::new(fill_width, snap.size.y)), || {
							yakui::colored_box(fg_color, Vec2::new(fill_width, snap.size.y));
						});
						// Optional label, centered on top of the bar
						if let Some(label) = &snap.text {
							yakui::constrained(Constraints::tight(snap.size), || {
								yakui::center(|| {
									yakui::text(snap.label_font_size.max(1.0), label.clone());
								});
							});
						}
					});
				}
				UiElement::Text(snap) => {
					text_entities.push(snap.entity);
					let font_size_changed =
						self.text_font_size_cache.insert(snap.entity, snap.font_size) != Some(snap.font_size);
					let text = if font_size_changed {
						// Zero-width space: invisible, but makes the text prop
						// differ from last frame's so yakui's cached widget
						// actually re-shapes with the new font_size.
						format!("{}\u{200B}", snap.text)
					} else {
						snap.text.clone()
					};

					yakui::offset(snap.screen_pos, || {
						yakui::constrained(Constraints::tight(snap.size), || {
							// yakui::text() is a shorthand for Text::new(), which only
							// sets font_size and leaves color at the TextStyle default
							// (white) — it has no color parameter. We need the widget
							// builder directly so we can mutate style.color.
							let mut text_widget = yakui::widgets::Text::new(snap.font_size, text);
							text_widget.style.color = yakui::Color::from_linear(Vec4::from_array(snap.color));
							text_widget.show();
						});
					});
				}
				UiElement::Image(snap) => {
					// No texture assigned yet, or it failed to load (already
					// logged once by `UiImageTextureCache::resolve`) -- draw
					// nothing rather than a placeholder box.
					if let Some((texture_id, uv_rect)) = snap.texture {
						let tint = yakui::Color::from_linear(Vec4::from_array(snap.color));
						let uv_rect =
							Rect::from_pos_size(Vec2::new(uv_rect[0], uv_rect[1]), Vec2::new(uv_rect[2], uv_rect[3]));

						yakui::offset(snap.screen_pos, || {
							yakui::constrained(Constraints::tight(snap.size), || {
								TexturedQuad::new(Some(texture_id), snap.size, tint, uv_rect).show();
							});
						});
					}
				}
			}
		}

		self.text_font_size_cache
			.retain(|entity, _| text_entities.contains(entity));

		manager.yak.finish();
		manager.mark_laid_out();

		// Pass 3: write interaction results back
		let world = scene.world_mut();
		for (entity, clicked, hovering) in button_results {
			if let Ok(mut button) = world.get::<&mut UIButton>(entity) {
				button.pressed = clicked;
				button.hovered = hovering;
			}
		}

		Ok(())
	}

	fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

#[cfg(test)]
mod tests {
	use ze_ecs::{EntityId, Scene};

	use super::{UIRect, Vec2, canvas_scale, effective_scale, resolve_screen_pos, ui_rect_size};
	use crate::components::{UIAnchorMode, UIAnchorPoint};

	/// The reference/design width the canvas scaler defaults to (1920x1080).
	const REF_W: f32 = 1920.0;

	fn rect(x: f32, y: f32, width: f32, height: f32, anchor_point: UIAnchorPoint) -> UIRect {
		UIRect {
			x,
			y,
			width,
			height,
			anchor_point,
		}
	}

	/// `TopLeft` (the default) must reproduce the original fixed-pixel-from-
	/// top-left behavior exactly, regardless of viewport size -- this is the
	/// backward-compatibility guarantee that lets every pre-existing scene
	/// (which never authored `anchor_point`) render identically after this
	/// change.
	#[test]
	fn top_left_anchor_ignores_viewport_size_both_directions() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(10.0, 20.0, 100.0, 30.0, UIAnchorPoint::TopLeft);

		for viewport_size in [
			Vec2::new(800.0, 600.0),
			Vec2::new(1920.0, 1080.0),
			Vec2::new(200.0, 150.0),
		] {
			let pos = resolve_screen_pos(
				&scene,
				None,
				viewport_size,
				entity,
				&r,
				UIAnchorMode::ScreenSpaceOverlay,
				1.0,
			);
			assert_eq!(pos, Vec2::new(10.0, 20.0));
		}
	}

	/// A `BottomRight`-anchored element must keep its bottom-right corner
	/// `x`/`y` pixels in from the viewport's bottom-right corner on both a
	/// larger and a smaller viewport -- i.e. it recomputes correctly in both
	/// resize directions, rather than drifting off-screen.
	#[test]
	fn bottom_right_anchor_tracks_viewport_size_growing_and_shrinking() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(-20.0, -10.0, 100.0, 30.0, UIAnchorPoint::BottomRight);

		let grown = resolve_screen_pos(
			&scene,
			None,
			Vec2::new(1920.0, 1080.0),
			entity,
			&r,
			UIAnchorMode::ScreenSpaceOverlay,
			1.0,
		);
		assert_eq!(grown, Vec2::new(1920.0 - 100.0 - 20.0, 1080.0 - 30.0 - 10.0));

		let shrunk = resolve_screen_pos(
			&scene,
			None,
			Vec2::new(640.0, 480.0),
			entity,
			&r,
			UIAnchorMode::ScreenSpaceOverlay,
			1.0,
		);
		assert_eq!(shrunk, Vec2::new(640.0 - 100.0 - 20.0, 480.0 - 30.0 - 10.0));
	}

	/// A `MiddleCenter`-anchored element must stay centered on the viewport
	/// (its own box center coinciding with the viewport center, plus the
	/// authored offset) as the viewport is resized.
	#[test]
	fn middle_center_anchor_centers_element_on_viewport() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(0.0, 0.0, 100.0, 40.0, UIAnchorPoint::MiddleCenter);

		let pos = resolve_screen_pos(
			&scene,
			None,
			Vec2::new(800.0, 600.0),
			entity,
			&r,
			UIAnchorMode::ScreenSpaceOverlay,
			1.0,
		);
		assert_eq!(pos, Vec2::new((800.0 - 100.0) / 2.0, (600.0 - 40.0) / 2.0));
	}

	/// `UIAnchorMode::WorldSpace` with no camera (or no transform) falls back
	/// to the same viewport-fraction anchoring as `ScreenSpaceOverlay`, so an
	/// element authored before ever having a camera in the scene still
	/// resizes correctly.
	#[test]
	fn world_space_without_camera_falls_back_to_viewport_anchor() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(-20.0, -10.0, 100.0, 30.0, UIAnchorPoint::BottomRight);

		let pos = resolve_screen_pos(
			&scene,
			None,
			Vec2::new(1920.0, 1080.0),
			entity,
			&r,
			UIAnchorMode::WorldSpace,
			1.0,
		);
		assert_eq!(pos, Vec2::new(1920.0 - 100.0 - 20.0, 1080.0 - 30.0 - 10.0));
	}

	/// Only `ScreenSpaceOverlay` elements pick up the match-by-width canvas
	/// scale; `WorldSpace` elements (pinned to a world object via camera
	/// projection) always stay at `1.0` so window width never distorts them.
	#[test]
	fn effective_scale_applies_only_to_screen_space_overlay() {
		assert_eq!(effective_scale(UIAnchorMode::ScreenSpaceOverlay, 2.0), 2.0);
		assert_eq!(effective_scale(UIAnchorMode::WorldSpace, 2.0), 1.0);
	}

	/// A `ScreenSpaceOverlay` rect authored at the reference resolution must
	/// scale both its offset from the anchor AND its own box size by the
	/// canvas factor, so its position and proportions track the viewport
	/// together. Here a 2x-wide viewport doubles a top-left element's offset
	/// and size.
	#[test]
	fn overlay_scale_multiplies_offset_and_size() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(10.0, 20.0, 100.0, 30.0, UIAnchorPoint::TopLeft);
		let scale = 2.0;

		let pos = resolve_screen_pos(
			&scene,
			None,
			Vec2::new(3840.0, 2160.0),
			entity,
			&r,
			UIAnchorMode::ScreenSpaceOverlay,
			scale,
		);
		assert_eq!(pos, Vec2::new(20.0, 40.0));
		assert_eq!(ui_rect_size(&r, scale), Vec2::new(200.0, 60.0));
	}

	/// With scaling, a `BottomRight`-anchored element keeps its scaled
	/// bottom-right corner the scaled offset in from the viewport's
	/// bottom-right corner: the anchor itself is not scaled (it's a live
	/// viewport position), but the element's scaled size and scaled offset are
	/// both subtracted/added, so the whole element grows toward the interior
	/// rather than drifting off-screen.
	#[test]
	fn overlay_scale_composes_with_bottom_right_anchor() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(-20.0, -10.0, 100.0, 30.0, UIAnchorPoint::BottomRight);
		let scale = 1.5;
		let viewport = Vec2::new(2880.0, 1620.0);

		let pos = resolve_screen_pos(
			&scene,
			None,
			viewport,
			entity,
			&r,
			UIAnchorMode::ScreenSpaceOverlay,
			scale,
		);
		assert_eq!(
			pos,
			Vec2::new(
				viewport.x - 100.0 * scale - 20.0 * scale,
				viewport.y - 30.0 * scale - 10.0 * scale,
			)
		);
	}

	/// The whole point of match-by-width: when the live viewport width equals
	/// the reference width, the scale factor is exactly `1.0`, so a scene
	/// authored/tested at the reference resolution renders identically after
	/// the scaler lands (even at a different height/aspect ratio).
	#[test]
	fn unit_scale_leaves_reference_resolution_untouched() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(15.0, 25.0, 120.0, 40.0, UIAnchorPoint::TopLeft);

		let pos = resolve_screen_pos(
			&scene,
			None,
			Vec2::new(1920.0, 1080.0),
			entity,
			&r,
			UIAnchorMode::ScreenSpaceOverlay,
			1.0,
		);
		assert_eq!(pos, Vec2::new(15.0, 25.0));
		assert_eq!(ui_rect_size(&r, 1.0), Vec2::new(120.0, 40.0));
	}

	/// `canvas_scale` must be derived from the *current* viewport width every
	/// time, giving `1.0` exactly at the reference width, `<1` below it, and
	/// `>1` above it -- across the full range, not just the two sizes an
	/// open-time check might sample.
	#[test]
	fn canvas_scale_tracks_viewport_width_across_the_range() {
		assert_eq!(canvas_scale(REF_W, REF_W), 1.0);
		assert_eq!(canvas_scale(REF_W, 960.0), 0.5);
		assert_eq!(canvas_scale(REF_W, 3840.0), 2.0);
		// Degenerate widths fall back to 1.0 rather than collapsing UI to zero.
		assert_eq!(canvas_scale(REF_W, 0.0), 1.0);
		assert_eq!(canvas_scale(0.0, REF_W), 1.0);
	}

	/// The regression this fixes: an edge/corner-anchored `ScreenSpaceOverlay`
	/// element must sit at the correct margin from its edge at *every* viewport
	/// size, not just at the reference size. The right/bottom margin (the gap
	/// between the element's far edge and the viewport's far edge) must equal
	/// the authored inset times the width-derived canvas scale -- and the
	/// element must never spill past the viewport edge -- for viewport widths
	/// smaller than, equal to, and larger than the reference width, and at any
	/// aspect ratio.
	#[test]
	fn bottom_right_margin_is_correct_at_every_viewport_size() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let inset = 24.0;
		let (w, h) = (150.0, 40.0);
		let r = rect(-inset, -inset, w, h, UIAnchorPoint::BottomRight);

		for viewport in [
			Vec2::new(1280.0, 720.0),  // smaller than reference
			Vec2::new(1920.0, 1080.0), // equal to reference
			Vec2::new(2560.0, 1440.0), // larger than reference
			Vec2::new(1080.0, 1920.0), // portrait / very different aspect
			Vec2::new(3840.0, 1080.0), // ultra-wide
		] {
			let scale = canvas_scale(REF_W, viewport.x);
			let pos = resolve_screen_pos(
				&scene,
				None,
				viewport,
				entity,
				&r,
				UIAnchorMode::ScreenSpaceOverlay,
				scale,
			);
			let size = ui_rect_size(&r, scale);
			let far = pos + size; // bottom-right corner of the element in screen px

			// Margin from the element's far edge to the viewport's far edge is
			// the scaled inset, on both axes.
			let expected_margin = inset * scale;
			assert!(
				(viewport.x - far.x - expected_margin).abs() < 1e-3,
				"right margin wrong at {viewport:?}: got {}, want {expected_margin}",
				viewport.x - far.x
			);
			assert!(
				(viewport.y - far.y - expected_margin).abs() < 1e-3,
				"bottom margin wrong at {viewport:?}: got {}, want {expected_margin}",
				viewport.y - far.y
			);
			// And it never spills off-screen.
			assert!(
				far.x <= viewport.x && far.y <= viewport.y,
				"element off-screen at {viewport:?}"
			);
		}
	}

	/// A `TopLeft`-anchored element's margin from the top-left corner is the
	/// authored offset times the canvas scale, at every viewport size.
	#[test]
	fn top_left_margin_scales_with_width_at_every_viewport_size() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(50.0, 30.0, 100.0, 20.0, UIAnchorPoint::TopLeft);

		for viewport in [
			Vec2::new(960.0, 540.0),
			Vec2::new(1920.0, 1080.0),
			Vec2::new(3840.0, 2160.0),
		] {
			let scale = canvas_scale(REF_W, viewport.x);
			let pos = resolve_screen_pos(
				&scene,
				None,
				viewport,
				entity,
				&r,
				UIAnchorMode::ScreenSpaceOverlay,
				scale,
			);
			assert!((pos.x - 50.0 * scale).abs() < 1e-3, "top-left x wrong at {viewport:?}");
			assert!((pos.y - 30.0 * scale).abs() < 1e-3, "top-left y wrong at {viewport:?}");
		}
	}

	/// A `MiddleCenter`-anchored element keeps its own center on the viewport
	/// center (plus its scaled offset) at every viewport size -- so a centered
	/// HUD element stays centered rather than drifting as the window resizes.
	#[test]
	fn middle_center_stays_centered_at_every_viewport_size() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let (w, h) = (200.0, 80.0);
		let r = rect(0.0, 0.0, w, h, UIAnchorPoint::MiddleCenter);

		for viewport in [
			Vec2::new(1024.0, 768.0),
			Vec2::new(1920.0, 1080.0),
			Vec2::new(3000.0, 1300.0),
		] {
			let scale = canvas_scale(REF_W, viewport.x);
			let pos = resolve_screen_pos(
				&scene,
				None,
				viewport,
				entity,
				&r,
				UIAnchorMode::ScreenSpaceOverlay,
				scale,
			);
			let size = ui_rect_size(&r, scale);
			let element_center = pos + size / 2.0;
			assert!(
				(element_center.x - viewport.x / 2.0).abs() < 1e-3,
				"not h-centered at {viewport:?}"
			);
			assert!(
				(element_center.y - viewport.y / 2.0).abs() < 1e-3,
				"not v-centered at {viewport:?}"
			);
		}
	}

	/// Models yakui's paint-time normalization of a layout-space position into
	/// `[0, 1]` surface coordinates, exactly as yakui-core's `PaintDom` does:
	/// `(pos * scale_factor + unscaled_viewport.pos()) / surface_size`. `1.0`
	/// on an axis means the point sits on the far (right/bottom) edge of the
	/// surface; `0.0` the near (left/top) edge. Values outside `[0, 1]` are
	/// off-screen.
	///
	/// This is what actually decides where a `ScreenSpaceOverlay` element ends
	/// up on screen -- the piece the earlier "the anchor math is correct" tests
	/// never modeled, which is why they passed while the UI was visibly
	/// misplaced.
	fn yakui_normalize(pos: Vec2, scale_factor: f32, viewport_origin: Vec2, surface_size: Vec2) -> Vec2 {
		(pos * scale_factor + viewport_origin) / surface_size
	}

	/// End-to-end: an edge/corner-anchored `ScreenSpaceOverlay` element with a
	/// zero inset (authored flush to the corner) must actually *paint* flush to
	/// that corner -- its far edge normalizing to `1.0` -- at every viewport
	/// size, given the layout frame `UISystem::update` now pins every frame
	/// (`scale_factor = 1.0`, zero viewport origin, `surface_size ==
	/// viewport`). This composes `canvas_scale` + `resolve_screen_pos` +
	/// yakui's paint normalization, so it fails if any of the three drifts out
	/// of the shared coordinate space -- unlike the pure anchor-math tests.
	#[test]
	fn overlay_corner_paints_flush_across_viewport_sizes() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(0.0, 0.0, 200.0, 60.0, UIAnchorPoint::BottomRight);

		for viewport in [
			Vec2::new(1280.0, 720.0),
			Vec2::new(1920.0, 1080.0),
			Vec2::new(2560.0, 1440.0),
			Vec2::new(1080.0, 1920.0),
			Vec2::new(3840.0, 1080.0),
		] {
			let scale = canvas_scale(REF_W, viewport.x);
			let pos = resolve_screen_pos(
				&scene,
				None,
				viewport,
				entity,
				&r,
				UIAnchorMode::ScreenSpaceOverlay,
				scale,
			);
			let far = pos + ui_rect_size(&r, scale);

			// The layout frame `UISystem` guarantees via `sync_layout_viewport`.
			let normalized = yakui_normalize(far, 1.0, Vec2::ZERO, viewport);
			assert!(
				(normalized.x - 1.0).abs() < 1e-4 && (normalized.y - 1.0).abs() < 1e-4,
				"corner not flush at {viewport:?}: normalized={normalized:?} (want ~1.0,1.0)"
			);
		}
	}

	/// Directly pins the failure mode this bug was: if yakui's paint scale
	/// factor is left at a non-1.0 OS value (what `yakui_winit`'s `auto_scale`
	/// used to do) while `resolve_screen_pos` keeps working in physical pixels,
	/// the same corner element is normalized well past the edge -- i.e. pushed
	/// off-screen by roughly the scale factor, and the error grows with the
	/// viewport. Proves the flush-placement test above is genuinely sensitive
	/// to the mismatch (a regression re-enabling OS scaling would trip it),
	/// and documents why pinning `scale_factor = 1.0` is required.
	#[test]
	fn nonunit_paint_scale_factor_pushes_corner_off_screen() {
		let scene = Scene::new("test");
		let entity = EntityId::dead();
		let r = rect(0.0, 0.0, 200.0, 60.0, UIAnchorPoint::BottomRight);
		let viewport = Vec2::new(2560.0, 1440.0);

		let scale = canvas_scale(REF_W, viewport.x);
		let pos = resolve_screen_pos(
			&scene,
			None,
			viewport,
			entity,
			&r,
			UIAnchorMode::ScreenSpaceOverlay,
			scale,
		);
		let far = pos + ui_rect_size(&r, scale);

		// Correct frame: flush.
		let good = yakui_normalize(far, 1.0, Vec2::ZERO, viewport);
		assert!((good.x - 1.0).abs() < 1e-4, "expected flush with scale 1.0");

		// A stray OS scale factor of 2.0 (pre-fix behavior) pushes the corner to
		// ~2.0 in normalized space -- far off the bottom-right of the surface.
		let bad = yakui_normalize(far, 2.0, Vec2::ZERO, viewport);
		assert!(
			bad.x > 1.9 && bad.y > 1.9,
			"non-unit scale should misplace, got {bad:?}"
		);
	}
}
