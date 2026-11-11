//! Every colour and metric used by the UI. Nothing else hardcodes either.

use std::sync::atomic::{AtomicU32, Ordering};

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Pixels, Rgba, SharedString, px, rgb, rgba};

pub const FONT: &str = "Segoe UI Variable Text";
pub const FONT_DISPLAY: &str = "Segoe UI Variable Display";
pub const FONT_MONO: &str = "Consolas";

pub const PANEL_GAP: Pixels = px(10.);
pub const PANEL_RADIUS: Pixels = px(16.);
pub const CARD_RADIUS: Pixels = px(12.);
pub const CONTROL_RADIUS: Pixels = px(10.);
/// Spotify's guidelines allow at most 8px on artwork.
pub const ART_RADIUS: Pixels = px(8.);
pub const ART_RADIUS_SMALL: Pixels = px(4.);

pub const SPOTIFY_ICON_SIZE: f32 = 22.;
pub const SPOTIFY_ICON_ASPECT: f32 = 236.05 / 225.25;
/// Space kept free beside the Spotify logo: half the icon's height.
pub const SPOTIFY_CLEAR_SPACE: f32 = SPOTIFY_ICON_SIZE / 2.;

/// The source switch: a pill with the two services in it.
pub const SWITCH_RADIUS: Pixels = px(12.);
pub const SWITCH_PADDING: f32 = 3.;
pub const SWITCH_GAP: f32 = 2.;
pub const SWITCH_SEGMENT_RADIUS: Pixels = px(9.);
pub const SWITCH_SEGMENT_HEIGHT: f32 = 34.;
pub const SWITCH_MARK: f32 = 20.;
/// Space around a segment's mark: raised, with the name beside it, and plain.
pub const SWITCH_RAISED_LEFT: f32 = 11.;
pub const SWITCH_RAISED_RIGHT: f32 = 14.;
pub const SWITCH_PLAIN_SIDE: f32 = 10.;
pub const SWITCH_PLAIN_WIDTH: f32 = 40.;
pub const SWITCH_NAME_GAP: f32 = 8.;
pub const SWITCH_NAME_SIZE: f32 = 15.;
/// The widest the name's box opens: enough for "YT Music" at its size.
pub const SWITCH_NAME_WIDTH: f32 = 76.;
/// How long the pill takes to move to the other service. The app restarts after it.
pub const SWITCH_DURATION: std::time::Duration = std::time::Duration::from_millis(260);

pub const SETTINGS_WIDTH: Pixels = px(720.);
pub const SETTINGS_CONTROL_WIDTH: Pixels = px(260.);
pub const LEGAL_WIDTH: Pixels = px(640.);
pub const LEGAL_HEIGHT: f32 = 0.8;

pub const SIDEBAR_WIDTH: Pixels = px(216.);
pub const RAIL_WIDTH: Pixels = px(64.);
pub const RIGHT_PANEL_WIDTH: Pixels = px(240.);
pub const PLAYER_HEIGHT: Pixels = px(84.);
/// The mini player's default size in compact mode and with the lyrics, the smallest it can be
/// resized to in each, the gap kept from the screen edge and the progress line.
pub const MINI_WIDTH: f32 = 360.;
pub const MINI_HEIGHT: f32 = 96.;
pub const MINI_LYRICS_HEIGHT: f32 = 300.;
pub const MINI_MIN_WIDTH: f32 = 300.;
pub const MINI_MIN_HEIGHT: f32 = 88.;
pub const MINI_LYRICS_MIN_HEIGHT: f32 = 200.;
pub const MINI_MARGIN: f32 = 16.;
pub const MINI_PROGRESS_HEIGHT: f32 = 2.;
/// How far inside the window edge a drag starts a resize.
pub const MINI_RESIZE_BAND: f32 = 6.;
/// The compact cover follows the window height up to this size; the lyrics header uses the small one.
pub const MINI_COVER_MAX: f32 = 120.;
pub const MINI_COVER_SMALL: f32 = 44.;
/// Space around the cover, and from the buttons to the window edge.
pub const MINI_PADDING: f32 = 12.;
/// Header buttons: one glyph size in one box size.
pub const MINI_HEADER_GLYPH: f32 = 14.;
pub const MINI_HEADER_BOX: f32 = 24.;
/// Transport buttons and the volume icon share the glyph size; the play button is a bit larger.
pub const MINI_TRANSPORT_GLYPH: f32 = 18.;
pub const MINI_TRANSPORT_BOX: f32 = 30.;
pub const MINI_PLAY_SIZE: f32 = 32.;
/// Empty margin inside the transport box around its glyph, so the left edge of previous lines up.
pub const MINI_PREVIOUS_INSET: f32 = (MINI_TRANSPORT_BOX - MINI_TRANSPORT_GLYPH) / 2.;
/// Empty margin inside the header box around its glyph, so the close glyph lines up with the slider.
pub const MINI_CLOSE_INSET: f32 = (MINI_HEADER_BOX - MINI_HEADER_GLYPH) / 2.;
/// The volume slider in the mini player's bottom row: its least width and how far it may grow.
pub const MINI_VOLUME_WIDTH: f32 = 64.;
pub const MINI_VOLUME_MAX_WIDTH: f32 = 140.;
/// The lyrics in the mini player.
pub const MINI_LYRICS_SIZE: f32 = 16.;
pub const MINI_LYRICS_GAP: f32 = 8.;
pub const MINI_LYRICS_END_PADDING: f32 = 96.;
/// The new output device popup: its window, the padding inside it, the gap between its
/// buttons and the gap kept above the mini player when that is open.
pub const OUTPUT_POPUP_WIDTH: f32 = 340.;
pub const OUTPUT_POPUP_HEIGHT: f32 = 136.;
pub const OUTPUT_POPUP_PADDING: f32 = 16.;
pub const OUTPUT_POPUP_ABOVE_MINI: f32 = 8.;
pub const OUTPUT_POPUP_MARK: f32 = 20.;
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

/// Lyrics text size and the space between lines.
pub const LYRICS_SIZE: f32 = 18.;
pub const LYRICS_GAP: f32 = 14.;
pub const LYRICS_SKELETON_HEIGHT: f32 = 18.;
pub const LYRICS_PLAIN_SIZE: f32 = 14.;
/// Room under the last line so it can scroll up to the focus point.
pub const LYRICS_END_PADDING: f32 = 240.;
/// Share of the lyrics list above the active line.
pub const LYRICS_FOCUS: f32 = 1. / 3.;

/// Saturation and lightness a cover's colour is clamped to, so it stays
/// readable on the dark panels and under dark text.
pub const ACCENT_SATURATION: (f32, f32) = (0.35, 0.75);
pub const ACCENT_LIGHTNESS: (f32, f32) = (0.70, 0.82);
/// How much lighter the hover state of a cover accent is.
const ACCENT_HOVER_LIFT: f32 = 0.06;
const HERO_TINT_ALPHA: f32 = 0.10;

/// The setup, uninstall and data-moving windows.
pub const WIZARD_WIDTH: f32 = 760.;
pub const WIZARD_HEIGHT: f32 = 480.;
pub const WIZARD_RAIL_WIDTH: Pixels = px(220.);
pub const WIZARD_TITLEBAR_HEIGHT: Pixels = px(36.);
pub const WIZARD_CLOSE_WIDTH: Pixels = px(46.);
pub const WIZARD_PADDING: f32 = 32.;
pub const WIZARD_TITLE_SIZE: f32 = 24.;
pub const WIZARD_LOGO: f32 = 56.;
pub const WIZARD_STATUS_ICON: f32 = 44.;
/// The circle beside each step and the check inside it.
pub const STEP_DOT: f32 = 18.;
pub const STEP_CHECK: f32 = 11.;
pub const PROGRESS_HEIGHT: Pixels = px(4.);
/// The guided setup column, and the extra space that separates its groups.
pub const SETUP_COLUMN_WIDTH: Pixels = px(520.);
pub const SETUP_GROUP_SPACE: Pixels = px(10.);
pub const SETUP_BADGE: f32 = 20.;
/// Distance of the source switch from the corner of the setup panel.
pub const SETUP_SWITCH_INSET: f32 = 16.;
pub const SETUP_LOG_HEIGHT: Pixels = px(180.);
pub const SETUP_OPTION_HEIGHT: Pixels = px(44.);

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

/// Packed `0xRRGGBB`, or `NO_ACCENT` while the lilac brand colour is used.
static ACCENT_OVERRIDE: AtomicU32 = AtomicU32::new(NO_ACCENT);
const NO_ACCENT: u32 = u32::MAX;

/// The lilac the logo always uses, whatever the current song tints.
pub fn brand_accent() -> Hsla {
    hsla(rgb(0xC9B2F5))
}

fn accent_override() -> Option<Hsla> {
    let packed = ACCENT_OVERRIDE.load(Ordering::Relaxed);

    (packed != NO_ACCENT).then(|| hsla(rgb(packed)))
}

/// Replaces the accent, or restores the brand colour with `None`. Returns
/// whether the colour changed.
pub fn set_accent(color: Option<Hsla>) -> bool {
    let packed = color.map_or(NO_ACCENT, |color| {
        let rgba = Rgba::from(color);
        let byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;

        (byte(rgba.r) << 16) | (byte(rgba.g) << 8) | byte(rgba.b)
    });

    ACCENT_OVERRIDE.swap(packed, Ordering::Relaxed) != packed
}

pub fn accent() -> Hsla {
    accent_override().unwrap_or_else(brand_accent)
}

pub fn accent_hover() -> Hsla {
    match accent_override() {
        Some(mut color) => {
            color.l = (color.l + ACCENT_HOVER_LIFT).min(1.);

            color
        }
        None => hsla(rgb(0xD9C8F9)),
    }
}

/// Faint wash of the accent over the Home hero card. Never drawn on artwork.
pub fn hero_tint() -> Hsla {
    let mut color = accent();

    color.a = HERO_TINT_ALPHA;

    color
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

/// Destructive actions, readable on the dark panels.
pub fn danger() -> Hsla {
    hsla(rgb(0xE5837F))
}

/// The Spotify logo and icon. Spotify's guidelines allow only the plain white
/// versions on dark backgrounds, never tinted.
pub fn spotify_mark() -> Hsla {
    hsla(rgb(0xFFFFFF))
}

/// The name in the source switch's raised segment.
pub fn switch_name() -> Hsla {
    hsla(rgb(0xFFFFFF))
}

/// A colour `amount` of the way from `from` to `to`.
pub fn mix(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    let amount = amount.clamp(0., 1.);
    let lerp = |a: f32, b: f32| a + (b - a) * amount;

    Hsla {
        h: lerp(from.h, to.h),
        s: lerp(from.s, to.s),
        l: lerp(from.l, to.l),
        a: lerp(from.a, to.a),
    }
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

/// Points the component library's accent-coloured parts at the current accent
/// and repaints every window, including cached views.
pub fn refresh_accent(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.colors.ring = accent();
        theme.colors.primary = accent();
        theme.colors.link = accent();
        theme.colors.slider_bar = accent();
    });
    cx.refresh_windows();
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
        theme.colors.link = accent();
        theme.colors.slider_bar = accent();
        theme.colors.slider_thumb = text_primary();
        theme.colors.scrollbar = transparent();
        theme.colors.scrollbar_thumb = hairline();
        theme.colors.scrollbar_thumb_hover = text_tertiary();
    });

    let _ = cx.theme();
}
