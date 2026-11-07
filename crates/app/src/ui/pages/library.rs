use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::{
    AnyElement, Context, Div, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, Size, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div, px, size,
};
use sonear_core::LibraryError;
use sonear_core::model::Page;

use super::empty::empty_state;
use crate::data::{self, Env, Load, Paged};
use crate::store::{AppStore, LibraryTab, Route};
use crate::theme;
use crate::ui::common::{
    CardItem, GRID_ROW, card_row, error_panel, grid_columns, map_page, skeleton_card,
    with_retry_bar,
};
use crate::ui::widgets::icon;

const TABS: [(LibraryTab, &str); 4] = [
    (LibraryTab::Playlists, "Playlists"),
    (LibraryTab::Albums, "Albums"),
    (LibraryTab::Artists, "Artists"),
    (LibraryTab::Shows, "Shows"),
];

#[derive(Default)]
struct TabState {
    items: Paged<CardItem>,
    started: bool,
}

pub struct LibraryPage {
    store: Entity<AppStore>,
    active: LibraryTab,
    tabs: HashMap<usize, TabState>,
    scroll: VirtualListScrollHandle,
    tasks: Vec<Task<()>>,
    _subscription: Subscription,
}

fn tab_index(tab: LibraryTab) -> usize {
    tab as usize
}

async fn fetch(
    core: &sonear_core::Core,
    tab: LibraryTab,
    offset: u32,
    cursor: Option<String>,
) -> Result<Page<CardItem>, LibraryError> {
    let library = &core.library;

    Ok(match tab {
        LibraryTab::Playlists => {
            map_page(library.my_playlists(offset).await?, |p| CardItem::from(&p))
        }
        LibraryTab::Albums => map_page(library.saved_albums(offset).await?, |a| CardItem::from(&a)),
        LibraryTab::Artists => map_page(library.followed_artists(cursor).await?, |a| {
            CardItem::from(&a)
        }),
        LibraryTab::Shows => map_page(library.saved_shows(offset).await?, |s| CardItem::from(&s)),
    })
}

impl LibraryPage {
    pub fn new(store: Entity<AppStore>, tab: LibraryTab, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            _subscription: cx.observe(&store, |_, _, cx| cx.notify()),
            store,
            active: tab,
            scroll: VirtualListScrollHandle::new(),
            tabs: HashMap::new(),
            tasks: Vec::new(),
        };

        page.open(tab, cx);

        page
    }

    fn env(&self, cx: &Context<Self>) -> Env {
        self.store.read(cx).env()
    }

    fn open(&mut self, tab: LibraryTab, cx: &mut Context<Self>) {
        self.active = tab;
        self.scroll = VirtualListScrollHandle::new();

        let state = self.tabs.entry(tab_index(tab)).or_default();

        if state.started {
            cx.notify();

            return;
        }

        state.started = true;
        self.start(tab, cx);
        cx.notify();
    }

    fn start(&mut self, tab: LibraryTab, cx: &mut Context<Self>) {
        let env = self.env(cx);
        let key = format!("library:{}:0", tab_index(tab));

        self.tasks.push(data::load(
            &env,
            Some(key),
            data::fetch(move |core| async move { fetch(&core, tab, 0, None).await }),
            cx,
            move |page: &mut Self, message, _| {
                let state = page.tabs.entry(tab_index(tab)).or_default();

                match message {
                    Load::Loading => {}
                    Load::Ready { value, .. } => state.items.set_first(value),
                    Load::Failed(error) => state.items.error = Some(error),
                }
            },
        ));
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let tab = self.active;

        self.tabs.insert(tab_index(tab), TabState::default());
        self.open(tab, cx);
    }

    fn load_more(&mut self, tab: LibraryTab, cx: &mut Context<Self>) {
        let state = self.tabs.entry(tab_index(tab)).or_default();
        let Some((offset, cursor)) = state.items.begin_next() else {
            return;
        };
        let env = self.env(cx);

        data::load_page(
            &env,
            move |core| Box::pin(async move { fetch(&core, tab, offset, cursor).await }),
            cx,
            move |page: &mut Self, result, _| {
                let state = page.tabs.entry(tab_index(tab)).or_default();

                match result {
                    Ok(next) => state.items.append(next),
                    Err(error) => state.items.fail(error),
                }
            },
        );
    }

    fn entry(&self, id: &'static str, name: IconName, label: &'static str, route: Route) -> Div {
        let store = self.store.clone();

        div().child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_3()
                .h(px(56.))
                .w(px(220.))
                .px_4()
                .rounded(theme::CARD_RADIUS)
                .bg(theme::raised())
                .border_1()
                .border_color(theme::transparent())
                .cursor_pointer()
                .tab_index(0)
                .hover(|style| style.bg(theme::active_nav()))
                .focus_visible(|style| style.border_color(theme::accent()))
                .child(icon(name, 18., theme::accent()))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text_primary())
                        .child(label),
                )
                .on_click(move |_, _, cx| {
                    store.update(cx, |state, cx| state.navigate(route.clone(), cx))
                }),
        )
    }

    fn tab_bar(&self, cx: &Context<Self>) -> Div {
        let page = cx.entity();

        div()
            .flex()
            .gap_2()
            .children(TABS.iter().map(|(tab, label)| {
                let tab = *tab;
                let page = page.clone();
                let active = self.active == tab;

                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "library-tab-{label}"
                    ))))
                    .px_4()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .cursor_pointer()
                    .tab_index(0)
                    .border_1()
                    .border_color(theme::hairline())
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .when_active(active)
                    .child(*label)
                    .on_click(move |_, _, cx| page.update(cx, |page, cx| page.open(tab, cx)))
            }))
    }

    fn grid(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.active;
        let state = self.tabs.entry(tab_index(tab)).or_default();

        if let Some(error) = &state.items.error
            && state.items.items.is_empty()
        {
            let this = cx.entity();

            return error_panel("library-retry", error, move |_, _, cx| {
                this.update(cx, |page, cx| page.retry(cx));
            });
        }

        if !state.items.first_loaded {
            return div()
                .flex()
                .gap(px(theme::GRID_GAP))
                .px_6()
                .children((0..4).map(|_| skeleton_card(theme::GRID_CARD)))
                .into_any_element();
        }

        if state.items.items.is_empty() {
            return empty_state(
                IconName::Library,
                match tab {
                    LibraryTab::Playlists => "No playlists yet",
                    LibraryTab::Albums => "No saved albums",
                    LibraryTab::Artists => "No followed artists",
                    LibraryTab::Shows => "No saved shows",
                },
                "What you save or follow in Spotify shows up here.",
            );
        }

        let columns = grid_columns(self.store.read(cx).main_width.get());
        let count = state.items.items.len();
        let rows = count.div_ceil(columns);
        let sizes: Rc<Vec<Size<Pixels>>> = Rc::new(vec![size(px(100.), px(GRID_ROW)); rows]);

        let more_error = state.items.error.clone();
        let this = cx.entity();
        let list = v_virtual_list(
            cx.entity(),
            ElementId::from(("library-grid", tab_index(tab))),
            sizes,
            move |this, range, _, cx| {
                let wants = {
                    let state = this.tabs.entry(tab_index(tab)).or_default();

                    state.items.wants_more(range.end * columns)
                };

                if wants {
                    this.load_more(tab, cx);
                }

                let store = this.store.clone();
                let state = this.tabs.entry(tab_index(tab)).or_default();

                range
                    .filter_map(|row| {
                        let start = row * columns;
                        let end = (start + columns).min(state.items.items.len());

                        (start < end).then(|| card_row(row, &state.items.items[start..end], &store))
                    })
                    .collect()
            },
        )
        .track_scroll(&self.scroll)
        .into_any_element();

        with_retry_bar(
            list,
            more_error.as_ref(),
            "library-more-retry",
            move |_, _, cx| {
                this.update(cx, |page, cx| {
                    if let Some(state) = page.tabs.get_mut(&tab_index(tab)) {
                        state.items.retry();
                    }

                    cx.notify();
                });
            },
        )
    }
}

trait ActiveStyle {
    fn when_active(self, active: bool) -> Self;
}

impl<T: gpui_kit::Styled> ActiveStyle for T {
    fn when_active(self, active: bool) -> Self {
        if active {
            self.bg(theme::accent()).text_color(theme::on_accent())
        } else {
            self.text_color(theme::text_secondary())
        }
    }
}

impl Render for LibraryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let grid = self.grid(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .flex_none()
                    .px_6()
                    .pt_6()
                    .pb_4()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child("Your Library"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(self.entry(
                                "entry-liked",
                                IconName::Heart,
                                "Liked Songs",
                                Route::LikedSongs,
                            ))
                            .child(self.entry(
                                "entry-history",
                                IconName::Clock,
                                "History",
                                Route::History,
                            )),
                    )
                    .child(self.tab_bar(cx)),
            )
            .child(div().flex_1().min_h_0().child(grid))
    }
}
