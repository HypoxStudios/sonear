use rust_i18n::t;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, IntoElement, ParentElement as _, Pixels, Render, Size,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px, size,
};
use sonear_core::HistoryRow;
use sonear_core::model::{Page, Track};
use tokio::sync::mpsc;

use super::empty::empty_state;
use crate::data::{self, Env, Paged};
use crate::format::played_at;
use crate::store::{AppStore, Confirm, Route};
use crate::theme;
use crate::ui::common::{
    ROW_HEIGHT, TrackRowSpec, breadcrumb, skeleton_rows, track_row, with_retry_bar,
};
use crate::ui::widgets::ghost_button;

const PAGE: u32 = 50;

pub struct HistoryPage {
    store: Entity<AppStore>,
    rows: Paged<HistoryRow>,
    scroll: VirtualListScrollHandle,
    tracks: HashMap<String, Track>,
    requested: HashSet<String>,
    seen_version: u64,
    tasks: Vec<Task<()>>,
    _subscription: Subscription,
}

async fn fetch_rows(core: &sonear_core::Core, offset: u32) -> Page<HistoryRow> {
    let rows = core.history.recent(PAGE, offset).await;
    let count = rows.len() as u32;

    Page {
        has_more: count == PAGE,
        items: rows,
        total: None,
        offset,
        cursor: None,
    }
}

impl HistoryPage {
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let seen_version = store.read(cx).history_version;
        let mut page = Self {
            _subscription: cx.observe(&store, |page, store, cx| {
                let version = store.read(cx).history_version;

                if version != page.seen_version {
                    page.seen_version = version;
                    page.reload(cx);
                }

                cx.notify();
            }),
            store,
            rows: Paged::default(),
            scroll: VirtualListScrollHandle::new(),
            tracks: HashMap::new(),
            requested: HashSet::new(),
            seen_version,
            tasks: Vec::new(),
        };

        page.reload(cx);

        page
    }

    fn env(&self, cx: &Context<Self>) -> Env {
        self.store.read(cx).env()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let env = self.env(cx);

        self.tasks = vec![data::load(
            &env,
            None,
            data::fetch(|core| async move { Ok(fetch_rows(&core, 0).await) }),
            cx,
            |page: &mut Self, message, _| {
                if let data::Load::Ready { value, .. } = message {
                    page.rows = Paged::default();
                    page.rows.set_first(value);
                }
            },
        )];
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some((offset, _)) = self.rows.begin_next() else {
            return;
        };
        let env = self.env(cx);

        data::load_page(
            &env,
            move |core| Box::pin(async move { Ok(fetch_rows(&core, offset).await) }),
            cx,
            |page: &mut Self, result, _| match result {
                Ok(next) => page.rows.append(next),
                Err(error) => page.rows.fail(error),
            },
        );
    }

    /// Looks up the details of rows on screen whose metadata is not known yet.
    fn resolve_visible(&mut self, uris: Vec<String>, cx: &mut Context<Self>) {
        let missing: Vec<String> = uris
            .into_iter()
            .filter(|uri| !self.tracks.contains_key(uri) && self.requested.insert(uri.clone()))
            .collect();

        if missing.is_empty() {
            return;
        }

        self.store.read(cx).ensure_likes(missing.clone());

        let env = self.env(cx);
        let (sender, mut inbox) = mpsc::unbounded_channel::<Track>();
        let history = env.core.history.clone();

        for uri in missing {
            let sender = sender.clone();
            let history = history.clone();

            env.runtime.spawn(async move {
                if let Some(track) = history.resolver().resolve(&uri).await {
                    let _ = sender.send(track);
                }
            });
        }

        drop(sender);
        cx.spawn(async move |this, cx| {
            while let Some(track) = inbox.recv().await {
                if this
                    .update(cx, |page, cx| {
                        page.tracks.insert(track.uri.clone(), track);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.rows.first_loaded {
            return div().px_5().child(skeleton_rows(9)).into_any_element();
        }

        if self.rows.items.is_empty() {
            return empty_state(
                IconName::RefreshCw,
                t!("history.empty.title"),
                t!("history.empty.body"),
            );
        }

        let sizes: Rc<Vec<Size<Pixels>>> =
            Rc::new(vec![size(px(100.), px(ROW_HEIGHT)); self.rows.items.len()]);

        let more_error = self.rows.error.clone();
        let this = cx.entity();
        let list = v_virtual_list(
            cx.entity(),
            "history-rows",
            sizes,
            move |this, range, _, cx| {
                if this.rows.wants_more(range.end) {
                    this.load_more(cx);
                }

                let wanted: Vec<String> = range
                    .clone()
                    .filter_map(|index| this.rows.items.get(index))
                    .filter(|row| row.track_uri.starts_with("spotify:track:"))
                    .map(|row| row.track_uri.clone())
                    .collect();

                this.resolve_visible(wanted, cx);

                let store = this.store.clone();
                let state = store.read(cx);
                let likes = state.core.likes.clone();
                let current = state.playback.item.as_ref().map(|item| item.uri.clone());

                range
                    .filter_map(|index| {
                        let row = this.rows.items.get(index)?;
                        let when = played_at(row.played_at_ms);

                        let Some(track) = this.tracks.get(&row.track_uri) else {
                            return Some(
                                div()
                                    .w_full()
                                    .px_5()
                                    .child(crate::ui::common::skeleton_row(index))
                                    .into_any_element(),
                            );
                        };
                        let spec = TrackRowSpec {
                            index,
                            track,
                            current: current.as_deref() == Some(track.uri.as_str()),
                            liked: Some(likes.is_liked(&track.uri).unwrap_or(false)),
                            show_art: true,
                            show_album: false,
                            show_number: false,
                            trailing: Some(when),
                            queue_store: Some(&store),
                            menu_store: Some(&store),
                        };
                        let play_store = store.clone();
                        let like_store = store.clone();
                        let uri = track.uri.clone();
                        let like_uri = track.uri.clone();

                        Some(track_row(
                            "history-row",
                            &spec,
                            move |_, _, cx| {
                                play_store.update(cx, |state, cx| {
                                    state.play_tracks(vec![uri.clone()], 0, cx)
                                });
                            },
                            move |_, _, cx| {
                                like_store.update(cx, |state, cx| state.toggle_like(&like_uri, cx));
                            },
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
            "history-more-retry",
            move |_, _, cx| {
                this.update(cx, |page, cx| {
                    page.rows.retry();
                    cx.notify();
                });
            },
        )
    }
}

impl Render for HistoryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let crumb = breadcrumb(&self.store, state, &Route::History.label());
        let store = self.store.clone();
        let has_rows = !self.rows.items.is_empty();
        let body = self.body(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(crumb)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_none()
                    .px_6()
                    .pb_3()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child(t!("route.history")),
                    )
                    .child(
                        ghost_button("history-clear", t!("history.clear"), has_rows).on_click(
                            move |_, _, cx| {
                                store.update(cx, |state, cx| state.ask(Confirm::ClearHistory, cx));
                            },
                        ),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}
