use rust_i18n::t;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollEvent, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px, relative,
};

use super::lyrics_feed::{Body, Follow, LyricsFeed};
use super::widgets::ghost_button;
use crate::store::AppStore;
use crate::theme;

const LRCLIB_URL: &str = "https://lrclib.net";
const SKELETON_WIDTHS: [f32; 7] = [0.9, 0.7, 0.8, 0.55, 0.85, 0.65, 0.75];
const NOTE: &str = "\u{266a}";

pub struct LyricsPanel {
    store: Entity<AppStore>,
    feed: Entity<LyricsFeed>,
    follow: Follow,
    _subscription: Subscription,
}

impl LyricsPanel {
    pub fn new(store: Entity<AppStore>, feed: Entity<LyricsFeed>, cx: &mut Context<Self>) -> Self {
        Self {
            _subscription: cx.observe(&feed, |_, _, cx| cx.notify()),
            store,
            feed,
            follow: Follow::new(),
        }
    }

    fn synced_list(&self, lines: &[sonear_core::LyricLine], cx: &Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let can_seek = state.playback.can("seek");
        let store = self.store.clone();
        let active = self.follow.active();

        div()
            .id("lyrics-lines")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(theme::LYRICS_GAP))
            .px_2()
            .pt_2()
            .pb(px(theme::LYRICS_END_PADDING))
            .overflow_y_scroll()
            .track_scroll(self.follow.scroll())
            .on_scroll(cx.listener(|panel, _: &ScrollEvent, _, _| {
                panel.follow.pause();
            }))
            .children(lines.iter().enumerate().map(|(index, line)| {
                let color = match active {
                    Some(active) if index == active => theme::text_primary(),
                    Some(active) if index < active => theme::text_secondary(),
                    _ => theme::text_tertiary(),
                };
                let text = if line.text.is_empty() {
                    NOTE
                } else {
                    line.text.as_str()
                };
                let at_ms = line.at_ms;
                let store = store.clone();

                div()
                    .id(ElementId::from(("lyric-line", index)))
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded(theme::CONTROL_RADIUS)
                    .text_size(px(theme::LYRICS_SIZE))
                    .font_weight(FontWeight::BOLD)
                    .font_family(theme::FONT_DISPLAY)
                    .text_color(color)
                    .child(text.to_owned())
                    .when(can_seek, |this| {
                        this.cursor_pointer()
                            .tab_index(0)
                            .hover(|style| style.bg(theme::hover()))
                            .focus_visible(|style| style.bg(theme::hover()))
                            .on_click(move |_, _, cx| {
                                store.update(cx, |store, cx| store.seek(at_ms, cx));
                            })
                    })
            }))
            .into_any_element()
    }

    fn plain_list(text: &str) -> AnyElement {
        div()
            .id("lyrics-plain")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .px_4()
            .pt_2()
            .pb_4()
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::text_tertiary())
                    .child(t!("lyrics.not_synced")),
            )
            .child(
                div()
                    .text_size(px(theme::LYRICS_PLAIN_SIZE))
                    .text_color(theme::text_primary())
                    .child(text.to_owned()),
            )
            .into_any_element()
    }
}

fn message(text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .px_4()
        .pt_2()
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child(text.into())
        .into_any_element()
}

fn skeleton() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(px(theme::LYRICS_GAP))
        .px_4()
        .pt_3()
        .children(SKELETON_WIDTHS.iter().map(|width| {
            div()
                .h(px(theme::LYRICS_SKELETON_HEIGHT))
                .w(relative(*width))
                .rounded(theme::ART_RADIUS_SMALL)
                .bg(theme::skeleton())
        }))
        .into_any_element()
}

impl Render for LyricsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let feed = self.feed.clone();
        let feed = feed.read(cx);

        self.follow.sync(feed);

        let repaint = self.follow.settle();
        let body = match feed.body() {
            Body::NothingPlaying => message(t!("lyrics.play_something")),
            Body::NotATrack => message(t!("lyrics.songs_only")),
            Body::Loading => skeleton(),
            Body::Synced(lines) => self.synced_list(lines, cx),
            Body::Plain(text) => Self::plain_list(text),
            Body::Instrumental => message(t!("lyrics.instrumental")),
            Body::NotFound => message(t!("lyrics.not_found")),
            Body::Failed => div()
                .flex()
                .flex_col()
                .items_start()
                .gap_3()
                .flex_1()
                .min_h_0()
                .px_4()
                .pt_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::text_secondary())
                        .child(t!("lyrics.load_failed")),
                )
                .child(
                    ghost_button("lyrics-retry", t!("common.try_again"), true).on_click({
                        let feed = self.feed.clone();

                        move |_, _, cx| feed.update(cx, |feed, cx| feed.retry(cx))
                    }),
                )
                .into_any_element(),
        };

        if repaint {
            cx.notify();
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .rounded(theme::PANEL_RADIUS)
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::panel())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_none()
                    .pl_4()
                    .pr_2()
                    .pt_4()
                    .pb_2()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child(t!("lyrics.title")),
                    ),
            )
            .child(body)
            .child(
                div()
                    .id("lyrics-credit")
                    .flex()
                    .items_center()
                    .flex_none()
                    .mx_4()
                    .mb_3()
                    .pt_3()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .cursor_pointer()
                    .tab_index(0)
                    .text_size(px(12.))
                    .text_color(theme::text_tertiary())
                    .hover(|style| style.text_color(theme::text_secondary()))
                    .focus_visible(|style| style.text_color(theme::accent()))
                    .child(t!("lyrics.credit"))
                    .on_click(|_, _, cx| cx.open_url(LRCLIB_URL)),
            )
    }
}
