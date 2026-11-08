use rust_i18n::t;
use std::rc::Rc;

use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, IntoElement, ParentElement as _, Pixels, Render, Size,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px, size,
};
use sonear_core::model::{Show, Track};

use crate::data::{self, Env, Load, Paged, Slot};
use crate::format::plain_text;
use crate::store::AppStore;
use crate::theme;
use crate::ui::common::{
    ROW_HEIGHT, TrackRowSpec, breadcrumb, error_panel, open_spotify, skeleton_rows, track_row,
    with_retry_bar,
};
use crate::ui::widgets::{accent_button, art, label_caps, pending_overlay, play_pause_icon};

pub struct ShowPage {
    store: Entity<AppStore>,
    id: String,
    show: Slot<Show>,
    episodes: Paged<Track>,
    scroll: VirtualListScrollHandle,
    tasks: Vec<Task<()>>,
    _subscription: Subscription,
}

impl ShowPage {
    pub fn new(store: Entity<AppStore>, id: String, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            _subscription: cx.observe(&store, |_, _, cx| cx.notify()),
            store,
            id,
            show: Slot::default(),
            episodes: Paged::default(),
            scroll: VirtualListScrollHandle::new(),
            tasks: Vec::new(),
        };

        page.start(cx);

        page
    }

    fn env(&self, cx: &Context<Self>) -> Env {
        self.store.read(cx).env()
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let env = self.env(cx);
        let (show_id, episodes_id) = (self.id.clone(), self.id.clone());

        self.tasks = vec![
            data::load(
                &env,
                Some(format!("show:{}", self.id)),
                data::fetch(move |core| async move { core.library.show(&show_id).await }),
                cx,
                |page: &mut Self, message, _| page.show.apply(message),
            ),
            data::load(
                &env,
                Some(format!("show-episodes:{}:0", self.id)),
                data::fetch(move |core| async move {
                    let page = core.library.show_episodes(&episodes_id, 0).await?;

                    Ok(crate::ui::common::map_page(page, Track::from))
                }),
                cx,
                |page: &mut Self, message, _| match message {
                    Load::Loading => {}
                    Load::Ready { value, .. } => page.episodes.set_first(value),
                    Load::Failed(error) => page.episodes.error = Some(error),
                },
            ),
        ];
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.show = Slot::default();
        self.episodes = Paged::default();
        self.start(cx);
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some((offset, _)) = self.episodes.begin_next() else {
            return;
        };
        let env = self.env(cx);
        let id = self.id.clone();

        data::load_page(
            &env,
            move |core| {
                Box::pin(async move {
                    let page = core.library.show_episodes(&id, offset).await?;

                    Ok(crate::ui::common::map_page(page, Track::from))
                })
            },
            cx,
            |page: &mut Self, result, _| match result {
                Ok(next) => page.episodes.append(next),
                Err(error) => page.episodes.fail(error),
            },
        );
    }

    fn header(&self, cx: &Context<Self>) -> AnyElement {
        let Some(show) = self.show.value() else {
            return div()
                .flex()
                .gap_5()
                .px_8()
                .py_4()
                .child(
                    div()
                        .size(px(140.))
                        .rounded(theme::ART_RADIUS)
                        .bg(theme::skeleton()),
                )
                .into_any_element();
        };
        let state = self.store.read(cx);
        let first = self
            .episodes
            .items
            .first()
            .map(|episode| episode.uri.clone());
        let playing = state.playback.is_playing();
        let pending = state.pending_play && !playing;
        let store = self.store.clone();
        let uris: Vec<String> = self.episodes.items.iter().map(|e| e.uri.clone()).collect();
        let play = {
            let mut button = accent_button(
                "show-play",
                if playing {
                    t!("common.pause")
                } else {
                    t!("common.play")
                },
                Some(play_pause_icon(playing, 14., theme::on_accent())),
                first.is_some(),
            )
            .relative();

            if pending {
                button = button.child(pending_overlay(theme::accent(), theme::on_accent()));
            }

            button.on_click(move |_, _, cx| {
                if playing {
                    store.read(cx).core.player.pause();
                } else {
                    store.update(cx, |state, cx| state.play_tracks(uris.clone(), 0, cx));
                }
            })
        };

        div()
            .flex()
            .items_center()
            .gap_6()
            .flex_none()
            .px_8()
            .py_4()
            .child(art(&show.art, &show.id, 140., theme::ART_RADIUS))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w_0()
                    .child(label_caps(t!("show.podcast")))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(36.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child(show.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme::text_secondary())
                            .child(show.publisher.clone()),
                    )
                    .child(
                        div()
                            .max_w(px(480.))
                            .line_clamp(2)
                            .text_size(px(12.))
                            .text_color(theme::text_tertiary())
                            .child(plain_text(&show.description)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(play)
                            .children(open_spotify("show-open", show.open_url.clone())),
                    ),
            )
            .into_any_element()
    }

    fn body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error) = self.episodes.error.clone()
            && self.episodes.items.is_empty()
        {
            let this = cx.entity();

            return error_panel("show-retry", &error, move |_, _, cx| {
                this.update(cx, |page, cx| page.retry(cx));
            });
        }

        if !self.episodes.first_loaded {
            return div().px_5().child(skeleton_rows(6)).into_any_element();
        }

        if self.episodes.items.is_empty() {
            return div()
                .p_8()
                .text_size(px(13.))
                .text_color(theme::text_secondary())
                .child(t!("show.no_episodes"))
                .into_any_element();
        }

        let sizes: Rc<Vec<Size<Pixels>>> = Rc::new(vec![
            size(px(100.), px(ROW_HEIGHT));
            self.episodes.items.len()
        ]);

        let more_error = self.episodes.error.clone();
        let this = cx.entity();
        let list = v_virtual_list(
            cx.entity(),
            "show-episodes",
            sizes,
            move |this, range, _, cx| {
                if this.episodes.wants_more(range.end) {
                    this.load_more(cx);
                }

                let store = this.store.clone();
                let current = store
                    .read(cx)
                    .playback
                    .item
                    .as_ref()
                    .map(|item| item.uri.clone());
                let page = cx.entity();

                range
                    .filter_map(|index| {
                        let episode = this.episodes.items.get(index)?;
                        let spec = TrackRowSpec {
                            index,
                            track: episode,
                            current: current.as_deref() == Some(episode.uri.as_str()),
                            liked: None,
                            show_art: true,
                            show_album: false,
                            show_number: false,
                            trailing: None,
                            queue_store: None,
                            menu_store: None,
                        };
                        let play_store = store.clone();
                        let play_page = page.clone();

                        Some(track_row(
                            "show-episode",
                            &spec,
                            move |_, _, cx| {
                                let uris = play_page
                                    .read(cx)
                                    .episodes
                                    .items
                                    .iter()
                                    .map(|episode| episode.uri.clone())
                                    .collect();

                                play_store
                                    .update(cx, |state, cx| state.play_tracks(uris, index, cx));
                            },
                            |_, _, _| {},
                        ))
                    })
                    .collect()
            },
        )
        .track_scroll(&self.scroll)
        .into_any_element();

        with_retry_bar(
            list,
            more_error.as_ref(),
            "show-more-retry",
            move |_, _, cx| {
                this.update(cx, |page, cx| {
                    page.episodes.retry();
                    cx.notify();
                });
            },
        )
    }
}

impl Render for ShowPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let name = self
            .show
            .value()
            .map_or_else(|| "\u{2026}".to_owned(), |show| show.name.clone());
        let crumb = breadcrumb(&self.store, state, &name);

        if let Some(error) = self.show.error().cloned() {
            let this = cx.entity();

            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(crumb)
                .child(error_panel("show-header-retry", &error, move |_, _, cx| {
                    this.update(cx, |page, cx| page.retry(cx));
                }))
                .into_any_element();
        }

        let header = self.header(cx);
        let body = self.body(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(crumb)
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
