use std::{cell::Cell, collections::HashMap};

use yakui::{Constraints, Vec2, Vec4};
use ze_core::Result;
use ze_ecs::{
	ActiveCameraView, EntityId, Scene, System,
	shipyard::{IntoIter, UniqueView, View},
};

use crate::{
	components::{UIBar, UIButton, UIRect, UIText},
	ui_manager::UiManagerHandle,
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

/// One paintable UI element, tagged with the data needed to order it against
/// every other element regardless of its component type.
enum UiElement {
	Button(ButtonSnapshot),
	Bar(BarSnapshot),
	Text(TextSnapshot),
}

impl UiElement {
	const fn sort_key(&self) -> (i32, EntityId) {
		match self {
			Self::Button(snap) => (snap.z_index, snap.entity),
			Self::Bar(snap) => (snap.z_index, snap.entity),
			Self::Text(snap) => (snap.z_index, snap.entity),
		}
	}
}

/// Resolves the screen-space pixel position for a UI rect. If the scene has
/// an active camera and the entity has a world transform, `rect.x/y` is
/// treated as a pixel offset from the entity's projected screen position.
/// Otherwise `rect.x/y` is treated as an absolute screen coordinate.
fn resolve_screen_pos(scene: &Scene, camera: Option<&ActiveCameraView>, entity: EntityId, rect: &UIRect) -> Vec2 {
	let anchor = camera.and_then(|camera| {
		scene
			.world_transform(entity)
			.and_then(|transform| camera.project_to_screen(transform.position))
	});

	anchor.map_or_else(
		|| Vec2::new(rect.x, rect.y),
		|anchor| Vec2::new(anchor.x + rect.x, anchor.y + rect.y),
	)
}

/// A negative width/height (e.g. from a mis-edited rect in the Inspector or
/// a hand-edited scene file) produces a negative `Constraints::tight` size.
/// yakui's layout isn't guaranteed to handle that gracefully, so clamp at
/// the source rather than let malformed data reach yakui at all.
const fn ui_rect_size(rect: &UIRect) -> Vec2 { Vec2::new(rect.width.max(0.0), rect.height.max(0.0)) }

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
}

impl UISystem {
	pub fn new(handle: UiManagerHandle) -> Self {
		Self {
			ui_manager: Some(handle),
			text_font_size_cache: HashMap::new(),
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

		let active_camera: Option<ActiveCameraView> = scene
			.world()
			.borrow::<UniqueView<ActiveCameraView>>()
			.ok()
			.map(|view| *view);

		// Pass 1: snapshot component data
		let button_snapshots: Vec<ButtonSnapshot> = {
			let world = scene.world();
			world.borrow::<View<UIButton>>().map_or_else(
				|_| Vec::new(),
				|buttons| {
					buttons
						.iter()
						.with_id()
						.map(|(entity, b)| ButtonSnapshot {
							entity,
							screen_pos: resolve_screen_pos(scene, active_camera.as_ref(), entity, &b.rect),
							size: ui_rect_size(&b.rect),
							text: b.text.clone(),
							font_size: b.font_size,
							color: b.color,
							hover_color: b.hover_color,
							pressed_color: b.pressed_color,
							z_index: b.z_index,
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
						.map(|(entity, b)| BarSnapshot {
							entity,
							screen_pos: resolve_screen_pos(scene, active_camera.as_ref(), entity, &b.rect),
							size: ui_rect_size(&b.rect),
							current: b.current,
							max: b.max,
							color: b.color,
							bg_color: b.bg_color,
							text: b.text.clone(),
							z_index: b.z_index,
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
						.map(|(entity, t)| TextSnapshot {
							entity,
							screen_pos: resolve_screen_pos(scene, active_camera.as_ref(), entity, &t.rect),
							size: ui_rect_size(&t.rect),
							text: t.text.clone(),
							// A non-positive font_size reaches cosmic-text as a negative
							// TextStyle::to_metrics() -> Metrics::line_height, which sends
							// Buffer::shape_until_scroll's line-layout loop into a hang
							// (confirmed via debugger: stuck in BufferLine::layout /
							// ShapeLine::layout_to_buffer). cosmic-text has no internal
							// guard against this, so clamp before it ever reaches yakui.
							font_size: t.font_size.max(1.0),
							color: t.color,
							z_index: t.z_index,
						})
						.collect()
				},
			)
		};

		let mut elements: Vec<UiElement> =
			Vec::with_capacity(button_snapshots.len() + bar_snapshots.len() + text_snapshots.len());
		elements.extend(button_snapshots.into_iter().map(UiElement::Button));
		elements.extend(bar_snapshots.into_iter().map(UiElement::Bar));
		elements.extend(text_snapshots.into_iter().map(UiElement::Text));
		elements.sort_by_key(UiElement::sort_key);

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
									yakui::text(BAR_LABEL_FONT_SIZE, label.clone());
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
			}
		}

		self.text_font_size_cache
			.retain(|entity, _| text_entities.contains(entity));

		manager.yak.finish();

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
