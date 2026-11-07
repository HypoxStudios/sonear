use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::widgets::{art, cover_for, icon};
use crate::store::{AppStore, UP_NEXT_ROWS};
use crate::theme;

pub fn right_panel(store: &Entity<AppStore>, state: &AppStore) -> AnyElement {
    let rows = state
        .playback
        .upcoming
        .iter()
        .take(UP_NEXT_ROWS)
        .map(|entry| {
            let item = &entry.item;
            let looked_up = state.queue_tracks.get(&item.uri);
            let found = looked_up.and_then(Option::as_ref);
            let (name, artists, known) = if !item.name().is_empty() {
                (item.name().to_owned(), item.artist_line(), true)
            } else if let Some(track) = found {
                (track.name.clone(), track.artist_line(), true)
            } else if looked_up.is_some() {
                ("Unknown track".to_owned(), String::new(), false)
            } else {
                ("Loading\u{2026}".to_owned(), String::new(), false)
            };
            let cover = match found.filter(|_| item.name().is_empty()) {
                Some(track) => art(&track.art, &track.uri, 40., px(8.)),
                None => cover_for(item.album().or(Some(item)), 40., px(8.)),
            };

            div()
                .flex()
                .items_center()
                .gap_3()
                .py(px(7.))
                .child(cover)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if known {
                                    theme::text_primary()
                                } else {
                                    theme::text_tertiary()
                                })
                                .child(name),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(theme::text_secondary())
                                .child(artists),
                        ),
                )
        });
    let empty = state.playback.upcoming.is_empty();

    let list = div()
        .id("up-next-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px_4()
        .children(rows)
        .when(empty, |this| {
            this.child(
                div()
                    .pt_2()
                    .text_size(px(12.))
                    .text_color(theme::text_tertiary())
                    .child("Nothing queued."),
            )
        });

    let footer = (!state.ai_hint_dismissed).then(|| {
        let store = store.clone();

        div()
            .id("ai-hint")
            .flex()
            .items_center()
            .gap_2()
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
            .child(icon(IconName::Sparkles, 13., theme::text_tertiary()))
            .child("AI controls available")
            .on_click(move |_, _, cx| store.update(cx, |state, cx| state.dismiss_ai_hint(cx)))
    });

    div()
        .flex()
        .flex_col()
        .flex_none()
        .h_full()
        .w(theme::RIGHT_PANEL_WIDTH)
        .min_h_0()
        .overflow_hidden()
        .rounded(theme::PANEL_RADIUS)
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::panel())
        .child(
            div()
                .flex_none()
                .px_4()
                .pt_4()
                .pb_2()
                .text_size(px(15.))
                .font_weight(FontWeight::BOLD)
                .font_family(theme::FONT_DISPLAY)
                .text_color(theme::text_primary())
                .child("Up next"),
        )
        .child(list)
        .children(footer)
        .into_any_element()
}
