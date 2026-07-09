use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

const EDITOR_FONT_NAME: &str = "JetBrains Mono Nerd Font";
const EDITOR_FONT_BYTES: &[u8] = include_bytes!("../resources/fonts/JetBrainsMono/JetBrainsMonoNerdFont-Regular.ttf");

// Background hierarchy — pure dark grays, no blue tint
pub const BG_BASE: egui::Color32 = egui::Color32::from_rgb(18, 18, 18); // deepest: window bg
pub const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(24, 24, 24); // panels
pub const BG_ELEVATED: egui::Color32 = egui::Color32::from_rgb(32, 32, 32); // inputs, headers
pub const BG_HOVER: egui::Color32 = egui::Color32::from_rgb(40, 40, 40); // hover state
pub const BG_ACTIVE: egui::Color32 = egui::Color32::from_rgb(48, 48, 48); // active/pressed

// Borders — subtle border matches the active background depth
pub const BORDER_SUBTLE: egui::Color32 = egui::Color32::from_rgb(48, 48, 48);
pub const BORDER_ACCENT: egui::Color32 = egui::Color32::from_rgb(83, 154, 252);

// Accent — blue appears only as accent, never as background fill
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(83, 154, 252);
pub const ACCENT_DIM: egui::Color32 = egui::Color32::from_rgb(40, 70, 130);

// Text
pub const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(210, 214, 222);
pub const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(130, 140, 160);

pub fn get_fonts() -> FontDefinitions {
	let mut fonts = FontDefinitions::default();
	fonts.font_data.insert(
		EDITOR_FONT_NAME.to_owned(),
		Arc::new(FontData::from_static(EDITOR_FONT_BYTES)),
	);

	// The editor uses this face first so text, code, and Nerd Font glyphs resolve
	// consistently.
	if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
		proportional.insert(0, EDITOR_FONT_NAME.to_owned());
	}
	if let Some(monospace) = fonts.families.get_mut(&FontFamily::Monospace) {
		monospace.insert(0, EDITOR_FONT_NAME.to_owned());
	}

	fonts
}

pub fn get_style() -> egui::Style {
	let mut style = egui::Style::default();

	style.spacing.item_spacing = egui::vec2(8.0, 8.0);
	style.spacing.window_margin = egui::Margin::same(12);
	style.spacing.button_padding = egui::vec2(10.0, 6.0);
	style.spacing.menu_margin = egui::Margin::same(8);
	style.spacing.indent = 5.0;
	style.spacing.interact_size = egui::vec2(40.0, 32.0);

	style.visuals = egui::Visuals::dark();

	style.visuals.override_text_color = Some(TEXT_PRIMARY);
	style.visuals.window_fill = BG_BASE;
	style.visuals.panel_fill = BG_PANEL;
	style.visuals.faint_bg_color = BG_ELEVATED;
	style.visuals.extreme_bg_color = BG_BASE;
	style.visuals.code_bg_color = BG_ELEVATED;

	style.visuals.window_stroke = egui::Stroke::new(1.0, BORDER_SUBTLE);

	style.visuals.widgets.noninteractive.bg_fill = BG_PANEL;
	style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER_SUBTLE);
	style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_MUTED);

	style.visuals.widgets.inactive.bg_fill = BG_ELEVATED;
	style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER_SUBTLE);
	style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

	style.visuals.widgets.hovered.bg_fill = BG_HOVER;
	style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER_ACCENT);
	style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

	style.visuals.widgets.active.bg_fill = BG_ACTIVE;
	style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
	style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

	style.visuals.widgets.open.bg_fill = BG_HOVER;
	style.visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, ACCENT);
	style.visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

	style.visuals.selection.bg_fill = ACCENT_DIM;
	style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);

	style.visuals.hyperlink_color = ACCENT;
	style.visuals.warn_fg_color = egui::Color32::from_rgb(255, 180, 50);
	style.visuals.error_fg_color = egui::Color32::from_rgb(255, 95, 95);

	style.visuals.window_corner_radius = egui::CornerRadius::same(10);
	style.visuals.menu_corner_radius = egui::CornerRadius::same(8);

	let corner = egui::CornerRadius::same(6);
	style.visuals.widgets.noninteractive.corner_radius = corner;
	style.visuals.widgets.inactive.corner_radius = corner;
	style.visuals.widgets.hovered.corner_radius = corner;
	style.visuals.widgets.active.corner_radius = corner;
	style.visuals.widgets.open.corner_radius = corner;

	style
}
