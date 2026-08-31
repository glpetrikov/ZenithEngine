use dear_app::imgui::{ColorOverride, StyleColor, StyleTweaks, TableTheme, Theme, ThemePreset, WindowTheme};

const BG_PRIMARY: [f32; 4] = [0.1216, 0.1216, 0.1216, 1.0];
const BG_SECONDARY: [f32; 4] = [0.1490, 0.1490, 0.1490, 1.0];
const BG_TERTIARY: [f32; 4] = [0.2196, 0.2196, 0.2196, 1.0];

const TEXT_PRIMARY: [f32; 4] = [0.9608, 0.9608, 0.9608, 1.0];
// pub const TEXT_ON_ACCENT: [f32; 4] = [0.1804, 0.1804, 0.1804, 1.0];
const TEXT_DISABLED: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

const ACCENT_DARKEST: [f32; 4] = [0.0784, 0.3020, 0.2039, 1.0];
const ACCENT_MUTED: [f32; 4] = [0.1059, 0.4314, 0.2902, 1.0];
const ACCENT_MUTED_HOVER: [f32; 4] = [0.1216, 0.4784, 0.3216, 1.0];
const ACCENT: [f32; 4] = [0.1333, 0.4902, 0.3333, 1.0];
const ACCENT_BRIGHT: [f32; 4] = [0.1804, 0.6118, 0.4196, 1.0];

// pub const COLOR_ERROR: [f32; 4] = [0.8275, 0.1843, 0.1843, 1.0];
// pub const COLOR_WARNING: [f32; 4] = [0.9608, 0.6510, 0.1373, 1.0];
// pub const COLOR_INFO: [f32; 4] = [0.2902, 0.5647, 0.8510, 1.0];
// pub const COLOR_SUCCESS: [f32; 4] = ACCENT;

// TODO: add others themes and Add the ability to add package-own editor themes
// to packages

// TODO: make it an package

pub fn zenith_theme() -> Theme {
	Theme {
		preset: ThemePreset::Dark,
		colors: vec![
			// Backgrounds
			ColorOverride {
				id: StyleColor::WindowBg,
				rgba: BG_PRIMARY,
			},
			ColorOverride {
				id: StyleColor::ChildBg,
				rgba: BG_PRIMARY,
			},
			ColorOverride {
				id: StyleColor::PopupBg,
				rgba: BG_SECONDARY,
			},
			ColorOverride {
				id: StyleColor::FrameBg,
				rgba: BG_SECONDARY,
			},
			ColorOverride {
				id: StyleColor::FrameBgHovered,
				rgba: BG_TERTIARY,
			},
			ColorOverride {
				id: StyleColor::FrameBgActive,
				rgba: BG_TERTIARY,
			},
			// Text
			ColorOverride {
				id: StyleColor::Text,
				rgba: TEXT_PRIMARY,
			},
			ColorOverride {
				id: StyleColor::TextDisabled,
				rgba: TEXT_DISABLED,
			},
			ColorOverride {
				id: StyleColor::TextSelectedBg,
				rgba: [ACCENT[0], ACCENT[1], ACCENT[2], 0.35],
			},
			// Borders
			ColorOverride {
				id: StyleColor::Border,
				rgba: BG_TERTIARY,
			},
			ColorOverride {
				id: StyleColor::BorderShadow,
				rgba: [0.0, 0.0, 0.0, 0.0],
			},
			// Buttons, checkboxes, sliders
			ColorOverride {
				id: StyleColor::Button,
				rgba: ACCENT,
			},
			ColorOverride {
				id: StyleColor::ButtonHovered,
				rgba: ACCENT_BRIGHT,
			},
			ColorOverride {
				id: StyleColor::ButtonActive,
				rgba: ACCENT_DARKEST,
			},
			ColorOverride {
				id: StyleColor::CheckMark,
				rgba: ACCENT_BRIGHT,
			},
			ColorOverride {
				id: StyleColor::SliderGrab,
				rgba: ACCENT,
			},
			ColorOverride {
				id: StyleColor::SliderGrabActive,
				rgba: ACCENT_DARKEST,
			},
			// Selection rows
			ColorOverride {
				id: StyleColor::Header,
				rgba: [ACCENT_MUTED[0], ACCENT_MUTED[1], ACCENT_MUTED[2], 0.35],
			},
			ColorOverride {
				id: StyleColor::HeaderHovered,
				rgba: [ACCENT_MUTED_HOVER[0], ACCENT_MUTED_HOVER[1], ACCENT_MUTED_HOVER[2], 0.6],
			},
			ColorOverride {
				id: StyleColor::HeaderActive,
				rgba: [ACCENT[0], ACCENT[1], ACCENT[2], 0.8],
			},
			// Resize grip
			ColorOverride {
				id: StyleColor::ResizeGrip,
				rgba: [BG_TERTIARY[0], BG_TERTIARY[1], BG_TERTIARY[2], 0.6],
			},
			ColorOverride {
				id: StyleColor::ResizeGripHovered,
				rgba: ACCENT_BRIGHT,
			},
			ColorOverride {
				id: StyleColor::ResizeGripActive,
				rgba: ACCENT_DARKEST,
			},
			// Title bars
			ColorOverride {
				id: StyleColor::TitleBg,
				rgba: BG_SECONDARY,
			},
			ColorOverride {
				id: StyleColor::TitleBgActive,
				rgba: BG_TERTIARY,
			},
			ColorOverride {
				id: StyleColor::TitleBgCollapsed,
				rgba: BG_PRIMARY,
			},
			// Tabs
			ColorOverride {
				id: StyleColor::Tab,
				rgba: BG_SECONDARY,
			},
			ColorOverride {
				id: StyleColor::TabHovered,
				rgba: ACCENT_BRIGHT,
			},
			ColorOverride {
				id: StyleColor::TabSelected,
				rgba: ACCENT,
			},
			ColorOverride {
				id: StyleColor::TabDimmed,
				rgba: BG_PRIMARY,
			},
			ColorOverride {
				id: StyleColor::TabDimmedSelected,
				rgba: BG_SECONDARY,
			},
			// Docking
			ColorOverride {
				id: StyleColor::DockingPreview,
				rgba: [ACCENT[0], ACCENT[1], ACCENT[2], 0.5],
			},
			ColorOverride {
				id: StyleColor::DockingEmptyBg,
				rgba: BG_PRIMARY,
			},
			// Scrollbar
			ColorOverride {
				id: StyleColor::ScrollbarBg,
				rgba: BG_PRIMARY,
			},
			ColorOverride {
				id: StyleColor::ScrollbarGrab,
				rgba: BG_TERTIARY,
			},
			ColorOverride {
				id: StyleColor::ScrollbarGrabHovered,
				rgba: ACCENT_BRIGHT,
			},
			ColorOverride {
				id: StyleColor::ScrollbarGrabActive,
				rgba: ACCENT_DARKEST,
			},
			// Separators
			ColorOverride {
				id: StyleColor::Separator,
				rgba: BG_TERTIARY,
			},
			ColorOverride {
				id: StyleColor::SeparatorHovered,
				rgba: ACCENT_BRIGHT,
			},
			ColorOverride {
				id: StyleColor::SeparatorActive,
				rgba: ACCENT_DARKEST,
			},
		],
		style: StyleTweaks {
			window_rounding: Some(6.0),
			child_rounding: Some(6.0),
			frame_rounding: Some(4.0),
			popup_rounding: Some(6.0),
			scrollbar_rounding: Some(3.0),
			grab_rounding: Some(4.0),
			tab_rounding: Some(6.0),
			// TODO: fill other fields
			..Default::default()
		},
		windows: WindowTheme::default(),
		tables: TableTheme::default(),
	}
}
