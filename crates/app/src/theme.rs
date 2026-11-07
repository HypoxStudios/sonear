//! Every colour and metric used by the UI. Nothing else hardcodes either.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Pixels, Rgba, SharedString, px, rgb, rgba};

pub const FONT: &str = "Segoe UI Variable Text";
pub const FONT_DISPLAY: &str = "Segoe UI Variable Display";

pub const PANEL_GAP: Pixels = px(10.);
pub const PANEL_RADIUS: Pixels = px(16.);
pub const CARD_RADIUS: Pixels = px(12.);
pub const CONTROL_RADIUS: Pixels = px(10.);
/// Spotify's guidelines allow at most 8px on artwork.
pub const ART_RADIUS: Pixels = px(8.);
pub const ART_RADIUS_SMALL: Pixels = px(4.);

pub const SPOTIFY_LOGO_HEIGHT: f32 = 20.;
pub const SPOTIFY_LOGO_ASPECT: f32 = 823.46 / 225.25;
pub const SPOTIFY_ICON_SIZE: f32 = 22.;
pub const SPOTIFY_ICON_ASPECT: f32 = 236.05 / 225.25;
/// Space kept free beside the Spotify logo: half the icon's height.
pub const SPOTIFY_CLEAR_SPACE: f32 = SPOTIFY_ICON_SIZE / 2.;

pub const SIDEBAR_WIDTH: Pixels = px(216.);
pub const RAIL_WIDTH: Pixels = px(64.);
pub const RIGHT_PANEL_WIDTH: Pixels = px(240.);
pub const PLAYER_HEIGHT: Pixels = px(84.);
pub const GRID_CARD: f32 = 168.;
pub const GRID_GAP: f32 = 18.;

/// Width of the command palette and the shortcut list.
pub const OVERLAY_WIDTH: Pixels = px(640.);
/// Share of the window height above the command palette.
pub const PALETTE_TOP: f32 = 0.18;
pub const PALETTE_MAX_HEIGHT: Pixels = px(440.);
/// Share of the window height the shortcut list may fill.
pub const SHORTCUTS_MAX_HEIGHT: f32 = 0.82;
pub const SHORTCUT_ROW_HEIGHT: Pixels = px(40.);
pub const PALETTE_COVER: f32 = 32.;

/// Window width below which the sidebar collapses to a rail.
pub const COLLAPSE_BELOW: f32 = 1100.;
/// Main plus right panel width below which the right panel hides.
pub const RIGHT_PANEL_MIN_SPAN: f32 = 900.;

fn hsla(color: Rgba) -> Hsla {
    color.into()
}

pub fn window() -> Hsla {
    hsla(rgb(0x0A090D))
}

pub fn panel() -> Hsla {
    hsla(rgb(0x17161C))
}

pub fn player() -> Hsla {
    hsla(rgb(0x18171D))
}

pub fn raised() -> Hsla {
    hsla(rgb(0x26232B))
}

pub fn active_nav() -> Hsla {
    hsla(rgb(0x2B2932))
}

pub fn hover() -> Hsla {
    hsla(rgba(0xFFFFFF0D))
}

pub fn accent() -> Hsla {
    hsla(rgb(0xC9B2F5))
}

pub fn accent_hover() -> Hsla {
    hsla(rgb(0xD9C8F9))
}

pub fn on_accent() -> Hsla {
    hsla(rgb(0x17121F))
}

pub fn text_primary() -> Hsla {
    hsla(rgb(0xF5F3F7))
}

pub fn text_secondary() -> Hsla {
    hsla(rgb(0x9D9AA6))
}

pub fn text_tertiary() -> Hsla {
    hsla(rgb(0x6E6B77))
}

pub fn hairline() -> Hsla {
    hsla(rgba(0xFFFFFF12))
}

pub fn warning() -> Hsla {
    hsla(rgb(0xE8C27A))
}

/// The Spotify logo and icon. Spotify's guidelines allow only the plain white
/// versions on dark backgrounds, never tinted.
pub fn spotify_mark() -> Hsla {
    hsla(rgb(0xFFFFFF))
}

/// Placeholder bars while content loads.
pub fn skeleton() -> Hsla {
    hsla(rgba(0xFFFFFF0F))
}

/// Dims the window behind a dialog.
pub fn scrim() -> Hsla {
    hsla(rgba(0x000000B3))
}

/// Chip behind a key binding that is waiting for new keys.
pub fn recording() -> Hsla {
    hsla(rgba(0xC9B2F51F))
}

pub fn transparent() -> Hsla {
    hsla(rgba(0x00000000))
}

pub fn art_outline() -> Hsla {
    hsla(rgba(0xFFFFFF55))
}

const ART_GRADIENTS: [(u32, u32); 8] = [
    (0xC9A27E, 0x3F4570),
    (0x8A6FB0, 0x2E2A4F),
    (0x8E7CB8, 0x35395C),
    (0xB57FA6, 0x3F3050),
    (0xBD7F72, 0x4A3552),
    (0x6F9BB3, 0x30405C),
    (0xB89A6A, 0x5A4033),
    (0x7C86C4, 0x2F2F58),
];

/// Two colours for a placeholder cover, stable for a given seed.
pub fn art_gradient(seed: &str) -> (Hsla, Hsla) {
    let hash = seed.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    let (from, to) = ART_GRADIENTS[(hash % ART_GRADIENTS.len() as u64) as usize];

    (hsla(rgb(from)), hsla(rgb(to)))
}

pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = SharedString::new_static(FONT);
        theme.colors.background = window();
        theme.colors.foreground = text_primary();
        theme.colors.muted_foreground = text_secondary();
        theme.colors.border = hairline();
        theme.colors.input = hairline();
        theme.colors.ring = accent();
        theme.colors.primary = accent();
        theme.colors.primary_foreground = on_accent();
        theme.colors.accent = active_nav();
        theme.colors.accent_foreground = text_primary();
        theme.colors.popover = raised();
        theme.colors.popover_foreground = text_primary();
        theme.colors.title_bar = window();
        theme.colors.title_bar_border = transparent();
        theme.colors.slider_bar = accent();
        theme.colors.slider_thumb = text_primary();
        theme.colors.scrollbar = transparent();
        theme.colors.scrollbar_thumb = hairline();
        theme.colors.scrollbar_thumb_hover = text_tertiary();
    });

    let _ = cx.theme();
}
