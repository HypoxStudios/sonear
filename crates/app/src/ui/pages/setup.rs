use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Div, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use sonear_core::AccountState;
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

const RAIL_STEPS: [&str; 5] = [
    "Agreement",
    "Spotify account",
    "Download Soloist",
    "Soloist key",
    "Pair device",
];

/// The `RAIL_STEPS` entry a step belongs to.
fn rail_index(step: SetupStep) -> usize {
    match step {
        SetupStep::Agreement => 0,
        SetupStep::Spotify => 1,
        SetupStep::Download => 2,
        SetupStep::Key => 3,
        SetupStep::Pair | SetupStep::Starting | SetupStep::Failed => 4,
    }
}

pub struct SetupView {
    store: Entity<AppStore>,
    key_input: Entity<InputState>,
    client_input: Entity<InputState>,
    copied: bool,
    show_connect_form: bool,
    _subscriptions: Vec<Subscription>,
}

impl SetupView {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Paste your Soloist key")
        });

        let saved_id = store.read(cx).core.config.get().spotify_client_id;
        let client_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Paste your Client ID");

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

        Self {
            _subscriptions: subscriptions,
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
                    if self.copied { "Copied" } else { "Copy" },
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| this.copy_redirect(cx))),
            )
    }

    fn legal_step(&self) -> AnyElement {
        let agreement = self.store.clone();
        let policy = self.store.clone();
        let accept = self.store.clone();

        column()
            .child(title("Before you start"))
            .child(point(
                "Sonear is independent. It isn't made or endorsed by Spotify.",
            ))
            .child(point(
                "Everything stays on this computer. Sonear sends nothing to its makers.",
            ))
            .child(point(
                "You need Spotify Premium, your own Spotify developer app and a Soloist key.",
            ))
            .child(spaced(
                div()
                    .flex()
                    .items_center()
                    .gap_5()
                    .child(text_link("read-terms", "End-user agreement").on_click(
                        move |_, _, cx| {
                            agreement.update(cx, |state, cx| state.open_legal(LegalDoc::Terms, cx));
                        },
                    ))
                    .child(text_link("read-privacy", "Privacy policy").on_click(
                        move |_, _, cx| {
                            policy.update(cx, |state, cx| state.open_legal(LegalDoc::Privacy, cx));
                        },
                    )),
            ))
            .child(spaced(note("By continuing you accept both documents.")))
            .child(
                accent_button("accept-legal", "Agree and continue", None::<Icon>, true).on_click(
                    move |_, _, cx| {
                        accept.update(cx, |state, cx| state.accept_legal(cx));
                    },
                ),
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
            .child(title("Connect Spotify"))
            .child(body(
                "Spotify requires every app that reads your library to use your own free \
                 developer app. It takes about two minutes and Sonear never sees your password.",
            ))
            .child(spaced(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w_full()
                    .child(instruction(
                        "1",
                        "Create an app in the Spotify dashboard. Any name works.",
                        Some(link(
                            "open-spotify-dashboard",
                            "Open the Spotify dashboard",
                            DASHBOARD_URL,
                        )),
                    ))
                    .child(instruction(
                        "2",
                        "Add this exact redirect URI to it:",
                        Some(self.redirect_box(cx)),
                    ))
                    .child(instruction(
                        "3",
                        "Tick \u{201c}Web API\u{201d} under the APIs the app will use.",
                        None::<Div>,
                    ))
                    .child(instruction(
                        "4",
                        "Paste the app's Client ID here:",
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
                accent_button("connect-spotify", "Connect Spotify", None::<Icon>, true)
                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
            ))
            .child(note(
                "Spotify Premium is required. The developer app is yours, for your own personal use. Your login is stored in Windows Credential Manager.",
            ))
            .into_any_element()
    }

    fn waiting_step(&self, url: String, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.clone();

        column()
            .child(title("Approve in your browser"))
            .child(body(
                "Sonear opened Spotify in your browser. Approve access there and this screen \
                 moves on by itself.",
            ))
            .child(spaced(spinner_row("Waiting for your approval")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        ghost_button("reopen-browser", "Open browser again", true)
                            .on_click(move |_, _, cx: &mut App| cx.open_url(&url)),
                    )
                    .child(ghost_button("cancel-connect", "Cancel", true).on_click(
                        move |_, _, cx| {
                            store.update(cx, |state, cx| state.cancel_connect(cx));
                        },
                    )),
            )
            .child(spaced(note(
                "If the browser shows \u{201c}Invalid redirect URI\u{201d}, add this address \
                 to your Spotify app's settings:",
            )))
            .child(self.redirect_box(cx))
            .into_any_element()
    }

    fn expired_step(&self, cx: &mut Context<Self>) -> AnyElement {
        column()
            .child(title("Log in again"))
            .child(body(
                "Spotify needs you to log in again. Your settings and Client ID are still here.",
            ))
            .child(spaced(
                accent_button("relogin", "Log in with Spotify", None::<Icon>, true)
                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
            ))
            .child(
                text_link("different-client-id", "Use a different Client ID").on_click(
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
            .child(title("Add your Soloist key"))
            .child(body(
                "Create a personal Soloist key in the Spotify developer dashboard. \
                 It needs Spotify Premium.",
            ))
            .child(link("open-dashboard", "Open the dashboard", DASHBOARD_URL))
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
                        accent_button("save-key", "Save key", None::<Icon>, true)
                            .on_click(cx.listener(|this, _, window, cx| this.save_key(window, cx))),
                    )
                    .children(can_cancel.then(|| {
                        ghost_button("cancel-key", "Cancel", true).on_click(move |_, _, cx| {
                            cancel.update(cx, |state, cx| state.cancel_edit_key(cx));
                        })
                    })),
            ))
            .child(note(
                "Your key is stored in Windows Credential Manager, not in a file.",
            ))
            .into_any_element()
    }

    fn pair_step(&self, device_name: &str) -> AnyElement {
        column()
            .child(title("Pair this computer"))
            .child(spaced(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w_full()
                    .child(instruction(
                        "1",
                        "Open Spotify on your phone, on the same Wi-Fi.",
                        None::<Div>,
                    ))
                    .child(instruction(
                        "2",
                        format!(
                            "Open the list of devices and choose \u{201c}{device_name}\u{201d}."
                        ),
                        None::<Div>,
                    )),
            ))
            .child(spaced(spinner_row("Waiting for pairing")))
            .child(note(
                "If Windows Firewall asks, allow Soloist on private networks.",
            ))
            .into_any_element()
    }

    fn failed_step(&self, reason: String, recent_log: Vec<String>) -> AnyElement {
        let retry = self.store.clone();
        let change = self.store.clone();

        column()
            .child(title("Soloist stopped"))
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
                            "Try again",
                            Some(icon(IconName::RefreshCw, 15., theme::on_accent())),
                            true,
                        )
                        .on_click(move |_, _, cx| {
                            retry.update(cx, |state, cx| state.retry(cx));
                        }),
                    )
                    .child(
                        ghost_button("change-key", "Change Soloist key", true).on_click(
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

fn point(text: &'static str) -> Div {
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
        .child(div().flex_1().child(text))
}

fn text_link(id: impl Into<ElementId>, text: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .tab_index(0)
        .text_size(px(13.))
        .text_color(theme::accent())
        .hover(|style| style.text_color(theme::accent_hover()))
        .focus_visible(|style| style.text_color(theme::text_primary()))
        .child(text)
}

fn link(id: impl Into<ElementId>, text: &'static str, url: &'static str) -> Stateful<Div> {
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
        .child(text)
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
        .child(title("Starting Soloist"))
        .child(spinner_row("One moment"))
        .into_any_element()
}

impl Render for SetupView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let setup = self.store.read(cx).setup_state();
        let step = setup.step();

        let content: AnyElement = match (step, &setup.status) {
            (SetupStep::Agreement, _) => self.legal_step(),
            (SetupStep::Spotify, _) => self.spotify_step(&setup, cx),
            (SetupStep::Key, _) => self.key_step(&setup, cx),
            (SetupStep::Download, Status::Expired) => download_step(
                &self.store,
                &setup.install,
                "This Soloist build has expired",
                "Spotify's player builds stop working after 90 days. Download the latest \
                 build to keep listening.",
                "Download update",
            ),
            (SetupStep::Download, _) => download_step(
                &self.store,
                &setup.install,
                "Set up playback",
                "Sonear plays music through Soloist, Spotify's own player. It is downloaded \
                 directly from Spotify, and only when you click the button below.",
                "Download Soloist",
            ),
            (SetupStep::Pair, Status::NeedsPairing { device_name }) => self.pair_step(device_name),
            (SetupStep::Failed, Status::Failed { reason, recent_log }) => {
                self.failed_step(reason.clone(), recent_log.clone())
            }
            _ => column()
                .child(title("Starting Soloist"))
                .child(spinner_row("One moment"))
                .into_any_element(),
        };

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
                            .child(chrome::step_list(&RAIL_STEPS, rail_index(step))),
                    ),
            )
            .child(panel().flex_1().min_w_0().child(scroll))
    }
}

fn download_step(
    store: &Entity<AppStore>,
    install: &InstallState,
    heading: &'static str,
    text: &'static str,
    button: &'static str,
) -> AnyElement {
    let busy = install.is_busy();
    let store = store.clone();
    let mut column = column().child(title(heading)).child(body(text)).child(link(
        "about-soloist",
        "About Soloist downloads",
        DOWNLOADS_URL,
    ));

    column = match install {
        InstallState::Downloading { received, total } => {
            let fraction = total
                .filter(|total| *total > 0)
                .map(|total| *received as f32 / total as f32);
            let label = match total {
                Some(total) => format!(
                    "Downloading {} of {}",
                    megabytes(*received),
                    megabytes(*total)
                ),
                None => format!("Downloading {}", megabytes(*received)),
            };

            column
                .child(spaced(chrome::progress_bar(fraction.unwrap_or(0.08))))
                .child(note(label))
        }
        InstallState::Verifying => column
            .child(spaced(chrome::progress_bar(1.)))
            .child(note("Checking that the download is signed by Spotify")),
        InstallState::Finalizing => column
            .child(spaced(chrome::progress_bar(1.)))
            .child(note("Installing")),
        InstallState::Idle | InstallState::Failed(_) => column,
    };

    if let InstallState::Failed(reason) = install {
        column = column.child(message(
            format!("The download was not installed: {reason}"),
            13.,
        ));
    }

    if !busy {
        let label = if matches!(install, InstallState::Failed(_)) {
            "Try again"
        } else {
            button
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
            (SetupStep::Agreement, 0),
            (SetupStep::Spotify, 1),
            (SetupStep::Download, 2),
            (SetupStep::Key, 3),
            (SetupStep::Pair, 4),
            (SetupStep::Starting, 4),
            (SetupStep::Failed, 4),
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
