use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Size, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px, size,
};
use sonear_core::QueueManager;
use sonear_soloist::protocol::Entity as SoloistEntity;

use super::widgets::{art, cover_for, icon, icon_button, spotify_icon};
use crate::store::{AppStore, UP_NEXT_ROWS};
use crate::theme;

const COVER: f32 = 40.;
const ROW_HEIGHT: f32 = 54.;
const HEADING_HEIGHT: f32 = 38.;
const NOTICE_HEIGHT: f32 = 84.;
const DROP_END_HEIGHT: f32 = 22.;
const EMPTY_HEIGHT: f32 = 30.;

const NOTICE: &str = "Playing on another device. Your queue picks up when playback returns here.";

/// What a dragged queue entry carries.
#[derive(Clone)]
struct QueueDrag {
    id: u64,
    title: SharedString,
}

struct DragPreview {
    title: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(220.))
            .truncate()
            .px_3()
            .py_2()
            .rounded(theme::CONTROL_RADIUS)
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::raised())
            .text_size(px(13.))
            .text_color(theme::text_primary())
            .child(self.title.clone())
    }
}

/// One line of the panel, in play order. Indices point into the store.
enum Row {
    Notice,
    /// The handed-off track before Soloist's queue shows it.
    Locked,
    /// An entry of Soloist's own queue, by index into `playback.upcoming`.
    Spotify(usize),
    Heading {
        title: SharedString,
        clear: bool,
    },
    Entry(usize),
    DropEnd,
    Context(usize),
    Empty,
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Self::Notice => NOTICE_HEIGHT,
            Self::Heading { .. } => HEADING_HEIGHT,
            Self::DropEnd => DROP_END_HEIGHT,
            Self::Empty => EMPTY_HEIGHT,
            Self::Locked | Self::Spotify(_) | Self::Entry(_) | Self::Context(_) => ROW_HEIGHT,
        }
    }
}

fn build_rows(state: &AppStore) -> Vec<Row> {
    let mut rows = Vec::new();
    let queue = &state.queue;
    let upcoming = &state.playback.upcoming;
    let has_own = !queue.entries.is_empty() || queue.locked.is_some();

    if has_own && !state.playback.is_active {
        rows.push(Row::Notice);
    }

    if let Some(locked) = &queue.locked
        && !upcoming
            .iter()
            .any(|entry| entry.source == "queue" && entry.item.uri == locked.uri)
    {
        rows.push(Row::Locked);
    }

    for (index, entry) in upcoming.iter().enumerate() {
        if entry.source == "queue" {
            rows.push(Row::Spotify(index));
        }
    }

    if !queue.entries.is_empty() {
        rows.push(Row::Heading {
            title: "In queue".into(),
            clear: true,
        });
        rows.extend((0..queue.entries.len()).map(Row::Entry));
        rows.push(Row::DropEnd);
    }

    let context: Vec<usize> = upcoming
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.source != "queue")
        .map(|(index, _)| index)
        .take(UP_NEXT_ROWS)
        .collect();

    if !context.is_empty() {
        let title = state
            .playback
            .context
            .as_ref()
            .map(|context| {
                if sonear_core::model::is_liked_context(&context.uri) {
                    "Liked Songs"
                } else {
                    context.name()
                }
            })
            .filter(|name| !name.is_empty())
            .unwrap_or("Then");

        rows.push(Row::Heading {
            title: title.to_owned().into(),
            clear: false,
        });
        rows.extend(context.into_iter().map(Row::Context));
    }

    if rows.is_empty() {
        rows.push(Row::Empty);
    }

    rows
}

/// What a row shows about its track.
struct Line {
    cover: AnyElement,
    name: String,
    artists: String,
    known: bool,
}

/// Names and covers come from Soloist when it has them, else from a lookup.
fn line_for_item(state: &AppStore, item: &SoloistEntity) -> Line {
    let looked_up = state.queue_tracks.get(&item.uri);
    let found = looked_up.and_then(Option::as_ref);

    if !item.name().is_empty() {
        return Line {
            cover: cover_for(Some(item), COVER, theme::ART_RADIUS_SMALL),
            name: item.name().to_owned(),
            artists: item.artist_line(),
            known: true,
        };
    }

    match found {
        Some(track) => Line {
            cover: art(&track.art, &track.uri, COVER, theme::ART_RADIUS_SMALL),
            name: track.name.clone(),
            artists: track.artist_line(),
            known: true,
        },
        None => Line {
            cover: cover_for(None, COVER, theme::ART_RADIUS_SMALL),
            name: if looked_up.is_some() {
                "Unknown track".to_owned()
            } else {
                "Loading\u{2026}".to_owned()
            },
            artists: String::new(),
            known: false,
        },
    }
}

fn line_for_uri(state: &AppStore, uri: &str) -> Line {
    let placeholder = SoloistEntity {
        uri: uri.to_owned(),
        ..SoloistEntity::default()
    };

    line_for_item(state, &placeholder)
}

fn tag(text: &'static str, color: gpui_kit::Hsla) -> AnyElement {
    div()
        .flex_none()
        .px_2()
        .py(px(1.))
        .rounded_full()
        .bg(theme::raised())
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(text)
        .into_any_element()
}

fn text_block(line: &Line) -> AnyElement {
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
                .text_color(if line.known {
                    theme::text_primary()
                } else {
                    theme::text_tertiary()
                })
                .child(line.name.clone()),
        )
        .child(
            div()
                .truncate()
                .text_size(px(12.))
                .text_color(theme::text_secondary())
                .child(line.artists.clone()),
        )
        .into_any_element()
}

fn plain_row(line: Line, trailing: Option<AnyElement>) -> AnyElement {
    let text = text_block(&line);

    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .h(px(ROW_HEIGHT))
        .px_4()
        .child(line.cover)
        .child(text)
        .children(trailing)
        .into_any_element()
}

pub struct RightPanel {
    store: Entity<AppStore>,
    scroll: VirtualListScrollHandle,
    _subscription: Subscription,
}

impl RightPanel {
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        Self {
            _subscription: cx.observe(&store, |_, _, cx| cx.notify()),
            store,
            scroll: VirtualListScrollHandle::new(),
        }
    }
}

fn entry_row(line: Line, id: u64, queue: Arc<QueueManager>) -> AnyElement {
    let play = queue.clone();
    let remove = queue.clone();
    let drop_queue = queue;
    let group = SharedString::from(format!("queue-entry-{id}"));
    let drag = QueueDrag {
        id,
        title: line.name.clone().into(),
    };
    let key = id as usize;
    let text = text_block(&line);

    div()
        .id(ElementId::from(("queue-entry", key)))
        .group(group.clone())
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .h(px(ROW_HEIGHT))
        .pl_2()
        .pr_4()
        .border_t_2()
        .border_color(theme::transparent())
        .cursor_pointer()
        .hover(|style| style.bg(theme::hover()))
        .on_click(move |_, _, _| play.play_now(id))
        .on_drag(drag, |drag, _, _, cx| {
            let title = drag.title.clone();

            cx.new(|_| DragPreview { title })
        })
        .drag_over::<QueueDrag>(|style, _, _, _| style.border_color(theme::accent()))
        .on_drop::<QueueDrag>(move |drag, _, _| drop_queue.move_before(drag.id, Some(id)))
        .child(icon(IconName::GripVertical, 14., theme::text_tertiary()))
        .child(line.cover)
        .child(text)
        .child(
            icon_button(
                ElementId::from(("queue-remove", key)),
                IconName::X,
                14.,
                26.,
                theme::text_secondary(),
                true,
            )
            .invisible()
            .group_hover(group, |style| style.visible())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                remove.remove(id);
            }),
        )
        .into_any_element()
}

impl Render for RightPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let rows = Rc::new(build_rows(state));
        let ai_hint = !state.ai_hint_dismissed;
        let sizes: Rc<Vec<Size<gpui_kit::Pixels>>> = Rc::new(
            rows.iter()
                .map(|row| size(px(100.), px(row.height())))
                .collect(),
        );
        let list = v_virtual_list(
            cx.entity(),
            "queue-rows",
            sizes,
            move |this, range, _, cx| {
                let store = this.store.clone();
                let wanted: Vec<String> = {
                    let state = store.read(cx);

                    range
                        .clone()
                        .filter_map(|index| rows.get(index))
                        .filter_map(|row| match row {
                            Row::Locked => state.queue.locked.as_ref().map(|t| t.uri.clone()),
                            Row::Entry(index) => {
                                state.queue.entries.get(*index).map(|t| t.uri.clone())
                            }
                            Row::Spotify(index) | Row::Context(index) => state
                                .playback
                                .upcoming
                                .get(*index)
                                .filter(|entry| entry.item.name().is_empty())
                                .map(|entry| entry.item.uri.clone()),
                            _ => None,
                        })
                        .filter(|uri| !state.queue_tracks.contains_key(uri))
                        .collect()
                };

                if !wanted.is_empty() {
                    store.update(cx, |state, cx| state.ensure_queue_rows(wanted, cx));
                }

                let state = store.read(cx);

                range
                    .filter_map(|index| {
                        Some(match rows.get(index)? {
                            Row::Notice => div()
                                .w_full()
                                .px_4()
                                .py_2()
                                .child(
                                    div()
                                        .p_3()
                                        .rounded(theme::CONTROL_RADIUS)
                                        .bg(theme::raised())
                                        .text_size(px(12.))
                                        .text_color(theme::text_secondary())
                                        .child(NOTICE),
                                )
                                .into_any_element(),
                            Row::Locked => {
                                let uri = &state.queue.locked.as_ref()?.uri;

                                plain_row(
                                    line_for_uri(state, uri),
                                    Some(tag("Up next", theme::accent())),
                                )
                            }
                            Row::Spotify(position) => {
                                let entry = state.playback.upcoming.get(*position)?;
                                let locked = state
                                    .queue
                                    .locked
                                    .as_ref()
                                    .is_some_and(|locked| locked.uri == entry.item.uri);
                                let label = if locked {
                                    tag("Up next", theme::accent())
                                } else {
                                    tag("From Spotify", theme::text_secondary())
                                };

                                plain_row(line_for_item(state, &entry.item), Some(label))
                            }
                            Row::Heading { title, clear } => {
                                let queue = state.core.queue.clone();

                                div()
                                    .flex()
                                    .items_end()
                                    .justify_between()
                                    .w_full()
                                    .h(px(HEADING_HEIGHT))
                                    .px_4()
                                    .pb_1()
                                    .child(
                                        div()
                                            .truncate()
                                            .min_w_0()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::BOLD)
                                            .font_family(theme::FONT_DISPLAY)
                                            .text_color(theme::text_primary())
                                            .child(title.clone()),
                                    )
                                    .when(*clear, |this| {
                                        this.child(
                                            div()
                                                .id("queue-clear")
                                                .flex_none()
                                                .cursor_pointer()
                                                .tab_index(0)
                                                .text_size(px(12.))
                                                .text_color(theme::text_tertiary())
                                                .hover(|style| {
                                                    style.text_color(theme::text_secondary())
                                                })
                                                .focus_visible(|style| {
                                                    style.text_color(theme::accent())
                                                })
                                                .child("Clear")
                                                .on_click(move |_, _, _| queue.clear()),
                                        )
                                    })
                                    .into_any_element()
                            }
                            Row::Entry(position) => {
                                let entry = state.queue.entries.get(*position)?;

                                entry_row(
                                    line_for_uri(state, &entry.uri),
                                    entry.id,
                                    state.core.queue.clone(),
                                )
                            }
                            Row::DropEnd => {
                                let queue = state.core.queue.clone();

                                div()
                                    .id("queue-drop-end")
                                    .h(px(DROP_END_HEIGHT))
                                    .mx_4()
                                    .border_t_2()
                                    .border_color(theme::transparent())
                                    .drag_over::<QueueDrag>(|style, _, _, _| {
                                        style.border_color(theme::accent())
                                    })
                                    .on_drop::<QueueDrag>(move |drag, _, _| {
                                        queue.move_before(drag.id, None);
                                    })
                                    .into_any_element()
                            }
                            Row::Context(position) => {
                                let entry = state.playback.upcoming.get(*position)?;

                                plain_row(line_for_item(state, &entry.item), None)
                            }
                            Row::Empty => div()
                                .px_4()
                                .pt_2()
                                .text_size(px(12.))
                                .text_color(theme::text_tertiary())
                                .child("Nothing queued.")
                                .into_any_element(),
                        })
                    })
                    .collect()
            },
        )
        .track_scroll(&self.scroll);

        let footer = ai_hint.then(|| {
            let store = self.store.clone();

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
                            .child("Up next"),
                    )
                    .child(spotify_icon()),
            )
            .child(div().flex_1().min_h_0().child(list))
            .children(footer)
    }
}
