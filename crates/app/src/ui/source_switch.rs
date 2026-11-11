//! The pill that shows which service Sonear plays from and switches to the
//! other one. The raised segment slides across while the app restarts.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Div, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, ease_in_out, px, svg,
};
use rust_i18n::t;
use sonear_core::Source;

use crate::assets;
use crate::store::AppStore;
use crate::theme;
use crate::ui::widgets::text_tooltip;

fn name(source: Source) -> String {
    match source {
        Source::Spotify => t!("source.name.spotify"),
        Source::Youtube => t!("source.name.youtube"),
    }
    .into_owned()
}

fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

/// A service's mark in the colour `color`. Spotify's is wider than tall.
fn mark(source: Source, group: &'static str, color: gpui_kit::Hsla, hover: bool) -> AnyElement {
    let (path, width) = match source {
        Source::Spotify => (
            assets::SPOTIFY_ICON,
            theme::SWITCH_MARK * theme::SPOTIFY_ICON_ASPECT,
        ),
        Source::Youtube => (assets::YTM_ICON, theme::SWITCH_MARK),
    };

    svg()
        .path(path)
        .flex_none()
        .w(px(width))
        .h(px(theme::SWITCH_MARK))
        .text_color(color)
        .when(hover, |this| {
            this.group_hover(group, |style| style.text_color(theme::text_primary()))
        })
        .into_any_element()
}

/// One segment, `raised` being 1 when it is the current service and 0 when
/// it is the plain button, and anything between while it moves.
fn segment(source: Source, raised: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(theme::SWITCH_SEGMENT_HEIGHT))
        .pl(px(lerp(
            theme::SWITCH_PLAIN_SIDE,
            theme::SWITCH_RAISED_LEFT,
            raised,
        )))
        .pr(px(lerp(
            theme::SWITCH_PLAIN_SIDE,
            theme::SWITCH_RAISED_RIGHT,
            raised,
        )))
        .gap(px(theme::SWITCH_NAME_GAP * raised))
        .rounded(theme::SWITCH_SEGMENT_RADIUS)
        .bg(theme::active_nav().opacity(raised))
        .child(mark(
            source,
            "",
            theme::mix(theme::text_secondary(), theme::switch_name(), raised),
            false,
        ))
        .child(
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .max_w(px(theme::SWITCH_NAME_WIDTH * raised))
                .opacity(raised)
                .text_size(px(theme::SWITCH_NAME_SIZE))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::switch_name())
                .child(name(source)),
        )
}

/// The plain button for the service that is not in use.
fn button(source: Source, store: &Entity<AppStore>) -> AnyElement {
    let store = store.clone();
    let group = match source {
        Source::Spotify => "source-switch-spotify",
        Source::Youtube => "source-switch-youtube",
    };
    let tip = match source {
        Source::Spotify => t!("source.switch_to_spotify"),
        Source::Youtube => t!("source.switch_to_youtube"),
    }
    .into_owned();

    div()
        .id(group)
        .group(group)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::SWITCH_SEGMENT_HEIGHT))
        .w(px(theme::SWITCH_PLAIN_WIDTH))
        .rounded(theme::SWITCH_SEGMENT_RADIUS)
        .border_1()
        .border_color(theme::transparent())
        .cursor_pointer()
        .tab_index(0)
        .hover(|style| style.bg(theme::hover()))
        .focus_visible(|style| style.border_color(theme::text_primary()))
        .tooltip(text_tooltip(tip))
        .child(mark(source, group, theme::text_secondary(), true))
        .on_click(move |_, _, cx| {
            store.update(cx, |state, cx| state.switch_source(source, cx));
        })
        .into_any_element()
}

/// The pill. Spotify is always the left segment and YT Music the right one.
pub fn source_switch(store: &Entity<AppStore>, current: Source, switching: Option<Source>) -> Div {
    let pill = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(theme::SWITCH_GAP))
        .p(px(theme::SWITCH_PADDING))
        .rounded(theme::SWITCH_RADIUS)
        .bg(theme::window())
        .border_1()
        .border_color(theme::hairline());

    if let Some(target) = switching.filter(|target| *target != current) {
        let moving = |source: Source| -> AnyElement {
            let (from, to) = if source == current {
                (1., 0.)
            } else {
                (0., 1.)
            };
            let id = ElementId::from((
                match source {
                    Source::Spotify => "source-move-spotify",
                    Source::Youtube => "source-move-youtube",
                },
                target as usize,
            ));

            div()
                .with_animation(
                    id,
                    Animation::new(theme::SWITCH_DURATION).with_easing(ease_in_out),
                    move |_, amount| segment(source, lerp(from, to, amount)),
                )
                .into_any_element()
        };

        return pill
            .child(moving(Source::Spotify))
            .child(moving(Source::Youtube));
    }

    let side = |source: Source| -> AnyElement {
        if source == current {
            segment(source, 1.).into_any_element()
        } else {
            button(source, store)
        }
    };

    pill.child(side(Source::Spotify))
        .child(side(Source::Youtube))
}
