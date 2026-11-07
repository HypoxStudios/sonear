use std::cell::RefCell;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, Div, ElementId, Entity as UiEntity, Hsla, InteractiveElement as _,
    IntoElement, ObjectFit, ParentElement as _, Pixels, SharedString, Stateful, Styled as _,
    StyledImage as _, Svg, Window, div, img, linear_color_stop, linear_gradient, px, svg,
};
use sonear_core::model::Artwork;
use sonear_soloist::protocol::Entity;

use crate::assets;
use crate::cover_cache::LruImageCache;
use crate::theme;

thread_local! {
    static IMAGE_CACHE: RefCell<Option<UiEntity<LruImageCache>>> = const { RefCell::new(None) };
}

/// Every cover decodes through this cache, which keeps a bounded number of images.
pub fn set_image_cache(cache: UiEntity<LruImageCache>) {
    IMAGE_CACHE.with(|slot| *slot.borrow_mut() = Some(cache));
}

pub fn icon(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

pub fn asset_icon(path: &'static str, size: f32, color: Hsla) -> Icon {
    Icon::default().path(path).size(px(size)).text_color(color)
}

pub fn play_pause_icon(playing: bool, size: f32, color: Hsla) -> Icon {
    let path = if playing {
        assets::PAUSE_FILLED
    } else {
        assets::PLAY_FILLED
    };

    asset_icon(path, size, color)
}

/// The logo in the accent colour. Below about 40 px the bars of the full mark
/// blur together, so small sizes use the simplified drawing.
pub fn logo_mark(size: f32) -> Icon {
    let path = if size < 40. {
        assets::MARK_SMALL
    } else {
        assets::MARK
    };

    asset_icon(path, size, theme::accent())
}

/// A square clickable icon with a visible keyboard focus ring. Attach
/// `on_click` to the result; disabled buttons ignore the pointer.
pub fn icon_button(
    id: impl Into<ElementId>,
    name: IconName,
    glyph: f32,
    box_size: f32,
    color: Hsla,
    enabled: bool,
) -> Stateful<Div> {
    button_with_icon(id, icon(name, glyph, color), box_size, enabled)
}

/// Outlined until liked, then filled in the accent colour.
pub fn heart_button(
    id: impl Into<ElementId>,
    liked: bool,
    glyph: f32,
    box_size: f32,
    color: Hsla,
    enabled: bool,
) -> Stateful<Div> {
    let heart = if liked {
        asset_icon(assets::HEART_FILLED, glyph, theme::accent())
    } else {
        icon(IconName::Heart, glyph, color)
    };

    button_with_icon(id, heart, box_size, enabled)
}

fn button_with_icon(
    id: impl Into<ElementId>,
    glyph: Icon,
    box_size: f32,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(px(box_size))
        .rounded(theme::CONTROL_RADIUS)
        .border_1()
        .border_color(theme::transparent())
        .child(glyph)
        .when(enabled, |this| {
            this.cursor_pointer()
                .tab_index(0)
                .hover(|style| style.bg(theme::hover()))
                .focus_visible(|style| style.border_color(theme::accent()))
        })
        .when(!enabled, |this| this.opacity(0.35))
}

pub fn round_play_button(
    id: impl Into<ElementId>,
    playing: bool,
    box_size: f32,
    fill: Hsla,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(px(box_size))
        .rounded_full()
        .bg(fill)
        .border_1()
        .border_color(theme::transparent())
        .child(play_pause_icon(
            playing,
            box_size * 0.45,
            theme::on_accent(),
        ))
        .when(enabled, |this| {
            this.cursor_pointer()
                .tab_index(0)
                .hover(|style| style.opacity(0.88))
                .focus_visible(|style| style.border_color(theme::accent()))
        })
        .when(!enabled, |this| this.opacity(0.5))
}

/// A quiet outlined button for secondary actions.
pub fn ghost_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(34.))
        .px_4()
        .rounded(theme::CONTROL_RADIUS)
        .border_1()
        .border_color(theme::hairline())
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child(label.into())
        .when(enabled, |this| {
            this.cursor_pointer()
                .tab_index(0)
                .hover(|style| style.bg(theme::hover()).text_color(theme::text_primary()))
                .focus_visible(|style| style.border_color(theme::accent()))
        })
        .when(!enabled, |this| this.opacity(0.4))
}

/// The full Spotify logo for the top row of pages that show Spotify content.
/// Only the sides carry the clear space; callers keep the rows above and below free.
pub fn spotify_logo() -> Svg {
    svg()
        .path(assets::SPOTIFY_LOGO)
        .flex_none()
        .w(px(theme::SPOTIFY_LOGO_HEIGHT * theme::SPOTIFY_LOGO_ASPECT))
        .h(px(theme::SPOTIFY_LOGO_HEIGHT))
        .mx(px(theme::SPOTIFY_CLEAR_SPACE))
        .text_color(theme::spotify_mark())
}

/// The Spotify icon, for places too small for the full logo.
pub fn spotify_icon() -> Svg {
    svg()
        .path(assets::SPOTIFY_ICON)
        .flex_none()
        .w(px(theme::SPOTIFY_ICON_SIZE * theme::SPOTIFY_ICON_ASPECT))
        .h(px(theme::SPOTIFY_ICON_SIZE))
        .mx(px(theme::SPOTIFY_CLEAR_SPACE))
        .text_color(theme::spotify_mark())
}

/// A tooltip builder showing `text`, for `.tooltip(...)` on a stateful element.
pub fn text_tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();

    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

pub fn accent_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    leading: Option<Icon>,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(36.))
        .px_4()
        .rounded(theme::CONTROL_RADIUS)
        .bg(theme::accent())
        .text_color(theme::on_accent())
        .text_sm()
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .border_1()
        .border_color(theme::transparent())
        .when_some(leading, |this, leading| this.child(leading))
        .child(label.into())
        .when(enabled, |this| {
            this.cursor_pointer()
                .tab_index(0)
                .hover(|style| style.bg(theme::accent_hover()))
                .focus_visible(|style| style.border_color(theme::text_primary()))
        })
        .when(!enabled, |this| this.opacity(0.5))
}

/// A cover image that reserves its size up front. Without a URL it draws a
/// stable gradient with the soft ellipse from the mockups; with one, the
/// gradient sits behind the image while it loads.
pub fn cover(url: Option<&str>, seed: &str, size: f32, radius: Pixels) -> AnyElement {
    let (from, to) = theme::art_gradient(seed);
    let base = div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(radius)
        .overflow_hidden()
        .bg(linear_gradient(
            135.,
            linear_color_stop(from, 0.),
            linear_color_stop(to, 1.),
        ));

    match url {
        Some(url) => {
            let mut image = img(url.to_owned());

            if let Some(cache) = IMAGE_CACHE.with(|slot| slot.borrow().clone()) {
                image = image.image_cache(&cache);
            }

            base.child(
                image
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element()
        }
        None => base
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(size * 0.42))
                    .h(px(size * 0.58))
                    .rounded_full()
                    .border_1()
                    .border_color(theme::art_outline()),
            )
            .into_any_element(),
    }
}

/// A cover from Spotify's catalogue, using the smallest image at least as large as `size`.
pub fn art(artwork: &Artwork, seed: &str, size: f32, radius: Pixels) -> AnyElement {
    cover(artwork.url_for(size.ceil() as u32), seed, size, radius)
}

/// A spinner centred over a filled round button, shown while a play request is pending.
pub fn pending_overlay(fill: Hsla, spinner: Hsla) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(fill)
        .child(Spinner::new().small().color(spinner))
}

pub fn cover_for(entity: Option<&Entity>, size: f32, radius: Pixels) -> AnyElement {
    let Some(entity) = entity else {
        return div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(size))
            .rounded(radius)
            .bg(theme::raised())
            .child(icon(IconName::Music, size * 0.36, theme::text_tertiary()))
            .into_any_element();
    };

    cover(
        entity.cover_url(preferred_size(size)),
        &entity.uri,
        size,
        radius,
    )
}

fn preferred_size(size: f32) -> &'static str {
    match size {
        s if s <= 64. => "small",
        s if s <= 200. => "default",
        _ => "large",
    }
}

pub fn label_caps(text: impl Into<SharedString>) -> Div {
    div()
        .text_xs()
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(theme::text_tertiary())
        .child(text.into().to_uppercase())
}

/// A rounded panel on the window background.
pub fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .min_h_0()
        .overflow_hidden()
        .rounded(theme::PANEL_RADIUS)
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::panel())
}
