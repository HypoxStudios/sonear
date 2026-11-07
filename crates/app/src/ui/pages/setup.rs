use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Div, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;
use sonear_core::AccountState;
use sonear_core::config::Language;
use sonear_soloist::Status;
use sonear_spotify::REDIRECT_URI;

use crate::format::megabytes;
use crate::installer::chrome;
use crate::legal::LegalDoc;
use crate::store::{AppStore, InstallState, SetupState, SetupStep};
use crate::theme;
use crate::ui::widgets::{accent_button, ghost_button, icon, panel};

pub const DOWNLOADS_URL: &str =
    "https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates";
pub const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

/// Translation keys of the rail labels.
const RAIL_STEPS: [&str; 6] = [
    "setup.rail.language",
    "setup.rail.agreement",
    "setup.rail.spotify",
    "setup.rail.download",
    "setup.rail.key",
    "setup.rail.pair",
];

/// The `RAIL_STEPS` entry a step belongs to.
fn rail_index(step: SetupStep) -> usize {
    match step {
        SetupStep::Language => 0,
        SetupStep::Agreement => 1,
        SetupStep::Spotify => 2,
        SetupStep::Download => 3,
        SetupStep::Key => 4,
        SetupStep::Pair | SetupStep::Starting | SetupStep::Failed => 5,
    }
}

pub struct SetupView {
    store: Entity<AppStore>,
    key_input: Entity<InputState>,
    client_input: Entity<InputState>,
    copied: bool,
    show_connect_form: bool,
    /// The language highlighted on the first step, shown but not saved until Continue.
    picked_language: Language,
    language_seen: u64,
    _subscriptions: Vec<Subscription>,
}

impl SetupView {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("common.key_placeholder"))
        });

        let saved_id = store.read(cx).core.config.get().spotify_client_id;
        let client_input = cx.new(|cx| {
            let mut input =
                InputState::new(window, cx).placeholder(t!("setup.connect.placeholder"));

            if !saved_id.is_empty() {
                input.set_value(saved_id, window, cx);
            }

            input
        });

        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&client_input, |this, _, event: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.connect(cx);
                }
            }),
            cx.subscribe_in(
                &key_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.save_key(window, cx);
                    }
                },
            ),
        ];

        let (picked_language, language_seen) = {
            let state = store.read(cx);

            (state.language(), state.language_version)
        };

        Self {
            _subscriptions: subscriptions,
            picked_language,
            language_seen,
            store,
            key_input,
            client_input,
            copied: false,
            show_connect_form: false,
        }
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        if matches!(self.store.read(cx).account, AccountState::LoggingIn { .. }) {
            return;
        }

        let id = self.client_input.read(cx).value().to_string();

        self.store.update(cx, |state, cx| state.connect(&id, cx));
    }

    fn copy_redirect(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(REDIRECT_URI.to_owned()));
        self.copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;

            let _ = this.update(cx, |setup, cx| {
                setup.copied = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn redirect_box(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .rounded(theme::CONTROL_RADIUS)
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::window())
                    .text_size(px(12.))
                    .font_family(theme::FONT_MONO)
                    .text_color(theme::text_primary())
                    .child(REDIRECT_URI),
            )
            .child(
                ghost_button(
                    "copy-redirect",
                    if self.copied {
                        t!("common.copied")
                    } else {
                        t!("common.copy")
                    },
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| this.copy_redirect(cx))),
            )
    }

    fn pick_language(&mut self, language: Language, cx: &mut Context<Self>) {
        self.picked_language = language;
        self.store
            .update(cx, |state, cx| state.preview_language(language, cx));
    }

    fn language_row(
        &self,
        index: usize,
        language: Language,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.picked_language == language;

        div()
            .id(ElementId::from((
                SharedString::from("language-option"),
                index,
            )))
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .h(theme::SETUP_OPTION_HEIGHT)
            .px_4()
            .rounded(theme::CONTROL_RADIUS)
            .border_1()
            .border_color(if selected {
                theme::accent()
            } else {
                theme::hairline()
            })
            .cursor_pointer()
            .tab_index(0)
            .text_size(px(14.))
            .text_color(theme::text_primary())
            .hover(|style| style.bg(theme::hover()))
            .focus_visible(|style| style.bg(theme::hover()).border_color(theme::text_primary()))
            .child(language.native_name())
            .when(selected, |this| {
                this.child(icon(IconName::Check, 16., theme::accent()))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.pick_language(language, cx)))
    }

    fn language_step(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.clone();
        let picked = self.picked_language;
        let rows: Vec<_> = Language::ALL
            .into_iter()
            .enumerate()
            .map(|(index, language)| self.language_row(index, language, cx))
            .collect();

        column()
            .child(title(t!("setup.language.title")))
            .child(body(t!("setup.language.body")))
            .child(spaced(
                div().flex().flex_col().gap_2().w_full().children(rows),
            ))
            .child(spaced(
                accent_button(
                    "language-continue",
                    t!("common.continue"),
                    None::<Icon>,
                    true,
                )
                .on_click(move |_, _, cx| {
                    store.update(cx, |state, cx| state.set_language(picked, cx));
                }),
            ))
            .into_any_element()
    }

    fn legal_step(&self) -> AnyElement {
        let agreement = self.store.clone();
        let policy = self.store.clone();
        let accept = self.store.clone();

        column()
            .child(title(t!("setup.legal.title")))
            .child(point(t!("setup.legal.independent")))
            .child(point(t!("setup.legal.local")))
            .child(point(t!("setup.legal.needs")))
            .child(spaced(
                div()
                    .flex()
                    .items_center()
                    .gap_5()
                    .child(text_link("read-terms", LegalDoc::Terms.title()).on_click(
                        move |_, _, cx| {
                            agreement.update(cx, |state, cx| state.open_legal(LegalDoc::Terms, cx));
                        },
                    ))
                    .child(
                        text_link("read-privacy", LegalDoc::Privacy.title()).on_click(
                            move |_, _, cx| {
                                policy.update(cx, |state, cx| {
                                    state.open_legal(LegalDoc::Privacy, cx)
                                });
                            },
                        ),
                    ),
            ))
            .child(spaced(note(t!("setup.legal.accept_note"))))
            .child(
                accent_button("accept-legal", t!("setup.legal.accept"), None::<Icon>, true)
                    .on_click(move |_, _, cx| {
                        accept.update(cx, |state, cx| state.accept_legal(cx));
                    }),
            )
            .into_any_element()
    }

    fn spotify_step(&self, setup: &SetupState, cx: &mut Context<Self>) -> AnyElement {
        match &setup.account {
            AccountState::LoggingIn { url } => self.waiting_step(url.clone(), cx),
            AccountState::SessionExpired if !self.show_connect_form => self.expired_step(cx),
            account => self.connect_form(account, cx),
        }
    }

    fn connect_form(&self, account: &AccountState, cx: &mut Context<Self>) -> AnyElement {
        let failure = match account {
            AccountState::LoginFailed(reason) => Some(reason.clone()),
            _ => None,
        };

        column()
            .child(title(t!("setup.connect.title")))
            .child(body(t!("setup.connect.body")))
            .child(spaced(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w_full()
                    .child(instruction(
                        "1",
                        t!("setup.connect.step1"),
                        Some(link(
                            "open-spotify-dashboard",
                            t!("setup.connect.open_dashboard"),
                            DASHBOARD_URL,
                        )),
                    ))
                    .child(instruction(
                        "2",
                        t!("setup.connect.step2"),
                        Some(self.redirect_box(cx)),
                    ))
                    .child(instruction("3", t!("setup.connect.step3"), None::<Div>))
                    .child(instruction(
                        "4",
                        t!("setup.connect.step4"),
                        Some(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .w_full()
                                .child(Input::new(&self.client_input))
                                .children(failure.map(|text| message(text, 13.))),
                        ),
                    )),
            ))
            .child(spaced(
                accent_button(
                    "connect-spotify",
                    t!("setup.connect.button"),
                    None::<Icon>,
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
            ))
            .child(note(t!("setup.connect.note")))
            .into_any_element()
    }

    fn waiting_step(&self, url: String, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.clone();

        column()
            .child(title(t!("setup.waiting.title")))
            .child(body(t!("setup.waiting.body")))
            .child(spaced(spinner_row(t!("setup.waiting.spinner"))))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        ghost_button("reopen-browser", t!("setup.waiting.reopen"), true)
                            .on_click(move |_, _, cx: &mut App| cx.open_url(&url)),
                    )
                    .child(
                        ghost_button("cancel-connect", t!("common.cancel"), true).on_click(
                            move |_, _, cx| {
                                store.update(cx, |state, cx| state.cancel_connect(cx));
                            },
                        ),
                    ),
            )
            .child(spaced(note(t!("setup.waiting.redirect_note"))))
            .child(self.redirect_box(cx))
            .into_any_element()
    }

    fn expired_step(&self, cx: &mut Context<Self>) -> AnyElement {
        column()
            .child(title(t!("setup.expired.title")))
            .child(body(t!("setup.expired.body")))
            .child(spaced(
                accent_button("relogin", t!("setup.expired.button"), None::<Icon>, true)
                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
            ))
            .child(
                text_link("different-client-id", t!("setup.expired.other_id")).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.show_connect_form = true;
                        cx.notify();
                    }),
                ),
            )
            .into_any_element()
    }

    fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key_input.read(cx).value().to_string();

        self.store.update(cx, |state, cx| state.save_key(key, cx));
        self.key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn key_step(&self, setup: &SetupState, cx: &mut Context<Self>) -> AnyElement {
        let cancel = self.store.clone();
        let can_cancel = setup.editing_key && !matches!(setup.status, Status::NeedsKey);

        column()
            .child(title(t!("setup.key.title")))
            .child(body(t!("setup.key.body")))
            .child(link(
                "open-dashboard",
                t!("setup.key.open_dashboard"),
                DASHBOARD_URL,
            ))
            .child(spaced(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .child(Input::new(&self.key_input).mask_toggle())
                    .children(setup.key_error.clone().map(|text| message(text, 12.))),
            ))
            .child(spaced(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        accent_button("save-key", t!("setup.key.save"), None::<Icon>, true)
                            .on_click(cx.listener(|this, _, window, cx| this.save_key(window, cx))),
                    )
                    .children(can_cancel.then(|| {
                        ghost_button("cancel-key", t!("common.cancel"), true).on_click(
                            move |_, _, cx| {
                                cancel.update(cx, |state, cx| state.cancel_edit_key(cx));
                            },
                        )
                    })),
            ))
            .child(note(t!("setup.key.note")))
            .into_any_element()
    }

    fn pair_step(&self, device_name: &str) -> AnyElement {
        column()
            .child(title(t!("setup.pair.title")))
            .child(spaced(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w_full()
                    .child(instruction("1", t!("setup.pair.step1"), None::<Div>))
                    .child(instruction(
                        "2",
                        t!("setup.pair.step2", device = device_name),
                        None::<Div>,
                    )),
            ))
            .child(spaced(spinner_row(t!("setup.pair.spinner"))))
            .child(note(t!("setup.pair.note")))
            .into_any_element()
    }

    fn failed_step(&self, reason: String, recent_log: Vec<String>) -> AnyElement {
        let retry = self.store.clone();
        let change = self.store.clone();

        column()
            .child(title(t!("setup.failed.title")))
            .child(body(reason))
            .child(
                div()
                    .id("soloist-log")
                    .w_full()
                    .h(theme::SETUP_LOG_HEIGHT)
                    .overflow_y_scroll()
                    .p_3()
                    .rounded(theme::CONTROL_RADIUS)
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::window())
                    .text_size(px(11.))
                    .font_family(theme::FONT_MONO)
                    .text_color(theme::text_secondary())
                    .children(recent_log.into_iter().map(|line| div().child(line))),
            )
            .child(spaced(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        accent_button(
                            "retry",
                            t!("common.try_again"),
                            Some(icon(IconName::RefreshCw, 15., theme::on_accent())),
                            true,
                        )
                        .on_click(move |_, _, cx| {
                            retry.update(cx, |state, cx| state.retry(cx));
                        }),
                    )
                    .child(
                        ghost_button("change-key", t!("setup.failed.change_key"), true).on_click(
                            move |_, _, cx| {
                                change.update(cx, |state, cx| state.edit_key(cx));
                            },
                        ),
                    ),
            ))
            .into_any_element()
    }
}

fn message(text: String, size: f32) -> Div {
    div()
        .text_size(px(size))
        .text_color(theme::warning())
        .child(text)
}

fn spaced(child: impl IntoElement) -> Div {
    div()
        .flex()
        .w_full()
        .pt(theme::SETUP_GROUP_SPACE)
        .child(child)
}

fn point(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_start()
        .gap_3()
        .w_full()
        .text_size(px(14.))
        .text_color(theme::text_secondary())
        .child(
            div()
                .flex_none()
                .mt(px(7.))
                .size(px(5.))
                .rounded_full()
                .bg(theme::accent()),
        )
        .child(div().flex_1().child(text.into()))
}

fn text_link(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .tab_index(0)
        .text_size(px(13.))
        .text_color(theme::accent())
        .hover(|style| style.text_color(theme::accent_hover()))
        .focus_visible(|style| style.text_color(theme::text_primary()))
        .child(text.into())
}

fn link(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    url: &'static str,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .tab_index(0)
        .text_size(px(13.))
        .text_color(theme::accent())
        .hover(|style| style.text_color(theme::accent_hover()))
        .focus_visible(|style| style.text_color(theme::text_primary()))
        .child(text.into())
        .child(icon(IconName::ExternalLink, 12., theme::accent()))
        .on_click(move |_, _, cx: &mut App| cx.open_url(url))
}

fn instruction(
    number: &'static str,
    text: impl Into<SharedString>,
    control: Option<impl IntoElement>,
) -> Div {
    div()
        .flex()
        .items_start()
        .gap_3()
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .size(px(theme::SETUP_BADGE))
                .rounded_full()
                .bg(theme::active_nav())
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text_primary())
                .child(number),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::text_secondary())
                        .child(text.into()),
                )
                .children(control),
        )
}

fn column() -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap_3()
        .w_full()
        .max_w(theme::SETUP_COLUMN_WIDTH)
}

fn title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(26.))
        .font_weight(FontWeight::BOLD)
        .font_family(theme::FONT_DISPLAY)
        .text_color(theme::text_primary())
        .child(text.into())
}

fn body(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(14.))
        .text_color(theme::text_secondary())
        .child(text.into())
}

fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(theme::text_tertiary())
        .child(text.into())
}

fn spinner_row(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child(Spinner::new().small().color(theme::text_secondary()))
        .child(text.into())
}

/// The small centred notice the normal shell shows while Soloist starts.
pub fn starting_notice() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .size_full()
        .child(title(t!("setup.starting.title")))
        .child(spinner_row(t!("setup.starting.spinner")))
        .into_any_element()
}

impl Render for SetupView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.store.read(cx).language_version;

        if language != self.language_seen {
            self.language_seen = language;
            self.key_input.update(cx, |input, cx| {
                input.set_placeholder(t!("common.key_placeholder"), window, cx);
            });
            self.client_input.update(cx, |input, cx| {
                input.set_placeholder(t!("setup.connect.placeholder"), window, cx);
            });
        }

        let setup = self.store.read(cx).setup_state();
        let step = setup.step();

        let content: AnyElement = match (step, &setup.status) {
            (SetupStep::Language, _) => self.language_step(cx),
            (SetupStep::Agreement, _) => self.legal_step(),
            (SetupStep::Spotify, _) => self.spotify_step(&setup, cx),
            (SetupStep::Key, _) => self.key_step(&setup, cx),
            (SetupStep::Download, Status::Expired) => download_step(
                &self.store,
                &setup.install,
                t!("setup.download.expired_title"),
                t!("setup.download.expired_body"),
                t!("common.download_update"),
            ),
            (SetupStep::Download, _) => download_step(
                &self.store,
                &setup.install,
                t!("setup.download.title"),
                t!("setup.download.body"),
                t!("setup.rail.download"),
            ),
            (SetupStep::Pair, Status::NeedsPairing { device_name }) => self.pair_step(device_name),
            (SetupStep::Failed, Status::Failed { reason, recent_log }) => {
                self.failed_step(reason.user_message(), recent_log.clone())
            }
            _ => column()
                .child(title(t!("setup.starting.title")))
                .child(spinner_row(t!("setup.starting.spinner")))
                .into_any_element(),
        };

        let rail_labels: Vec<SharedString> = RAIL_STEPS
            .iter()
            .map(|key| SharedString::from(t!(*key).into_owned()))
            .collect();

        let scroll = div().id("setup").size_full().overflow_y_scroll().child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .min_h_full()
                .p_8()
                .child(content),
        );

        div()
            .flex()
            .size_full()
            .gap(theme::PANEL_GAP)
            .child(
                panel()
                    .w(theme::WIZARD_RAIL_WIDTH)
                    .flex_none()
                    .p_6()
                    .child(chrome::rail_header())
                    .child(
                        div()
                            .pt_8()
                            .child(chrome::step_list(&rail_labels, rail_index(step))),
                    ),
            )
            .child(panel().flex_1().min_w_0().child(scroll))
    }
}

fn download_step(
    store: &Entity<AppStore>,
    install: &InstallState,
    heading: impl Into<SharedString>,
    text: impl Into<SharedString>,
    button: impl Into<SharedString>,
) -> AnyElement {
    let busy = install.is_busy();
    let store = store.clone();
    let mut column = column().child(title(heading)).child(body(text)).child(link(
        "about-soloist",
        t!("setup.download.about"),
        DOWNLOADS_URL,
    ));

    column = match install {
        InstallState::Downloading { received, total } => {
            let fraction = total
                .filter(|total| *total > 0)
                .map(|total| *received as f32 / total as f32);
            let label = match total {
                Some(total) => t!(
                    "install.downloading_of",
                    received = megabytes(*received),
                    total = megabytes(*total)
                ),
                None => t!("install.downloading", received = megabytes(*received)),
            };

            column
                .child(spaced(chrome::progress_bar(fraction.unwrap_or(0.08))))
                .child(note(label))
        }
        InstallState::Verifying => column
            .child(spaced(chrome::progress_bar(1.)))
            .child(note(t!("install.verifying"))),
        InstallState::Finalizing => column
            .child(spaced(chrome::progress_bar(1.)))
            .child(note(t!("install.installing"))),
        InstallState::Idle | InstallState::Failed(_) => column,
    };

    if let InstallState::Failed(reason) = install {
        column = column.child(message(
            t!("install.failed", reason = reason).into_owned(),
            13.,
        ));
    }

    if !busy {
        let label: SharedString = if matches!(install, InstallState::Failed(_)) {
            t!("common.try_again").into()
        } else {
            button.into()
        };

        column = column.child(spaced(
            accent_button(
                "download-soloist",
                label,
                Some(icon(IconName::Download, 15., theme::on_accent())),
                true,
            )
            .on_click(move |_, _, cx| store.update(cx, |state, cx| state.start_install(cx))),
        ));
    }

    column.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_index_follows_the_step_order() {
        let steps = [
            (SetupStep::Language, 0),
            (SetupStep::Agreement, 1),
            (SetupStep::Spotify, 2),
            (SetupStep::Download, 3),
            (SetupStep::Key, 4),
            (SetupStep::Pair, 5),
            (SetupStep::Starting, 5),
            (SetupStep::Failed, 5),
        ];

        for (step, index) in steps {
            assert_eq!(rail_index(step), index);
        }
    }

    #[test]
    fn last_step_is_the_last_rail_entry() {
        assert_eq!(rail_index(SetupStep::Failed), RAIL_STEPS.len() - 1);
    }
}
