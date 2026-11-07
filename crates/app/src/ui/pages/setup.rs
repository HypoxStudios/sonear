use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Div, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px, relative,
};
use sonear_core::AccountState;
use sonear_soloist::Status;
use sonear_spotify::REDIRECT_URI;

use crate::format::megabytes;
use crate::legal::LegalDoc;
use crate::store::{AppStore, InstallState};
use crate::theme;
use crate::ui::widgets::{accent_button, ghost_button, icon, logo_mark};

pub const DOWNLOADS_URL: &str =
    "https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates";
pub const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

pub struct SetupView {
    store: Entity<AppStore>,
    key_input: Entity<InputState>,
    client_input: Entity<InputState>,
    copied: bool,
    _subscription: Subscription,
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

        Self {
            _subscription: cx.observe(&store, |_, _, cx| cx.notify()),
            store,
            key_input,
            client_input,
            copied: false,
        }
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
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

    fn legal_step(&self) -> AnyElement {
        let agreement = self.store.clone();
        let policy = self.store.clone();
        let accept = self.store.clone();

        card()
            .child(logo_mark(56.))
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
            .child(
                text_link("read-terms", "Read the end-user agreement").on_click(move |_, _, cx| {
                    agreement.update(cx, |state, cx| state.open_legal(LegalDoc::Terms, cx));
                }),
            )
            .child(
                text_link("read-privacy", "Read the privacy policy").on_click(move |_, _, cx| {
                    policy.update(cx, |state, cx| state.open_legal(LegalDoc::Privacy, cx));
                }),
            )
            .child(
                accent_button("accept-legal", "Agree and continue", None::<Icon>, true).on_click(
                    move |_, _, cx| {
                        accept.update(cx, |state, cx| state.accept_legal(cx));
                    },
                ),
            )
            .child(note("By continuing you accept both documents."))
            .into_any_element()
    }

    fn connect_step(&self, account: &AccountState, cx: &mut Context<Self>) -> AnyElement {
        let waiting = matches!(account, AccountState::LoggingIn { .. });
        let store = self.store.clone();
        let mut column = card()
            .child(logo_mark(56.))
            .child(title("Connect Spotify"))
            .child(body(
                "Spotify requires every app that reads your library to use your own free \
                 developer app. It takes about two minutes and Sonear never sees your password.",
            ))
            .child(link(
                "open-spotify-dashboard",
                "Open the Spotify dashboard",
                DASHBOARD_URL,
            ))
            .child(step("1", "Create an app there. Any name works."))
            .child(step("2", "Add this exact redirect URI to it:"))
            .child(
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
                            .font_family("Consolas")
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
                    ),
            )
            .child(step(
                "3",
                "Tick \u{201c}Web API\u{201d} under the APIs the app will use.",
            ))
            .child(step("4", "Paste the app's Client ID here:"))
            .child(
                div()
                    .w_full()
                    .child(Input::new(&self.client_input).disabled(waiting)),
            );

        column = if waiting {
            column
                .child(spinner_row("Waiting for you to approve in the browser"))
                .child(note(
                    "If the browser shows \u{201c}Invalid redirect URI\u{201d}, the address above is \
                     missing from your Spotify app's settings.",
                ))
                .child(
                    ghost_button("cancel-connect", "Cancel", true).on_click(move |_, _, cx| {
                        store.update(cx, |state, cx| state.cancel_connect(cx));
                    }),
                )
        } else {
            column.child(
                accent_button("connect-spotify", "Connect Spotify", None::<Icon>, true)
                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
            )
        };

        let message = match account {
            AccountState::LoginFailed(reason) => Some(reason.clone()),
            AccountState::SessionExpired => Some("Spotify needs you to log in again.".to_owned()),
            _ => None,
        };

        column
            .children(message.map(|text| {
                div()
                    .text_size(px(13.))
                    .text_color(theme::warning())
                    .child(text)
            }))
            .child(note(
                "Spotify Premium is required. The developer app is yours, for your own personal use. Your login is stored in Windows Credential Manager.",
            ))
            .into_any_element()
    }

    fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key_input.read(cx).value().to_string();

        self.store.update(cx, |state, cx| state.save_key(key, cx));
        self.key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn key_step(&self, key_error: Option<String>, cx: &mut Context<Self>) -> AnyElement {
        card()
            .child(title("Add your Soloist key"))
            .child(body(
                "Create a personal Soloist key in the Spotify developer dashboard. \
                 It needs Spotify Premium.",
            ))
            .child(link("open-dashboard", "Open the dashboard", DASHBOARD_URL))
            .child(
                div()
                    .w_full()
                    .pt_2()
                    .child(Input::new(&self.key_input).mask_toggle()),
            )
            .child(
                accent_button("save-key", "Save key", None::<Icon>, true)
                    .on_click(cx.listener(|this, _, window, cx| this.save_key(window, cx))),
            )
            .children(key_error.map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(theme::warning())
                    .child(error)
            }))
            .child(note(
                "Your key is stored in Windows Credential Manager, not in a file.",
            ))
            .into_any_element()
    }
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

fn link(id: impl Into<ElementId>, text: &'static str, url: &'static str) -> impl IntoElement {
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

/// Demo builds can open the Connect step for screenshots with `SONEAR_DEMO_SETUP=1`.
#[cfg(feature = "demo")]
fn preview_connect() -> bool {
    std::env::var("SONEAR_DEMO_SETUP").is_ok()
}

#[cfg(not(feature = "demo"))]
fn preview_connect() -> bool {
    false
}

fn step(number: &'static str, text: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap_3()
        .w_full()
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .size(px(20.))
                .rounded_full()
                .bg(theme::active_nav())
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text_primary())
                .child(number),
        )
        .child(div().flex_1().child(text))
}

fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap_3()
        .w_full()
        .max_w(px(520.))
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

fn progress_bar(fraction: Option<f32>) -> Div {
    let fill = div()
        .h_full()
        .rounded_full()
        .bg(theme::accent())
        .w(relative(fraction.unwrap_or(0.08).clamp(0., 1.)));

    div()
        .w_full()
        .h(px(6.))
        .rounded_full()
        .bg(theme::hairline())
        .child(fill)
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

impl Render for SetupView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let status = state.soloist.clone();
        let install = state.install.clone();
        let key_error = state.key_error.clone();
        let editing_key = state.editing_key;
        let account = state.account.clone();
        let connected = state.connected();
        let legal_accepted = state.legal_accepted();
        let store = self.store.clone();

        let content: AnyElement = match status {
            _ if !legal_accepted => self.legal_step(),
            _ if !connected || preview_connect() => self.connect_step(&account, cx),
            _ if editing_key => self.key_step(key_error, cx),
            Status::NeedsKey => self.key_step(key_error, cx),
            Status::NotInstalled => download_step(
                &store,
                &install,
                "Set up playback",
                "Sonear plays music through Soloist, Spotify's own player. It is downloaded \
                 directly from Spotify, and only when you click the button below.",
                "Download Soloist",
            ),
            Status::Expired => download_step(
                &store,
                &install,
                "This Soloist build has expired",
                "Spotify's player builds stop working after 90 days. Download the latest \
                 build to keep listening.",
                "Download update",
            ),
            Status::NeedsPairing { device_name } => card()
                .child(title("Pair this computer"))
                .child(body(format!(
                    "Open Spotify on your phone on the same Wi-Fi and choose \u{201c}{device_name}\u{201d} \
                     from the list of devices."
                )))
                .child(spinner_row("Waiting for pairing"))
                .child(note(
                    "If Windows Firewall asks, allow Soloist on private networks.",
                ))
                .into_any_element(),
            Status::Failed { reason, recent_log } => card()
                .child(title("Soloist stopped"))
                .child(body(reason))
                .child(
                    div()
                        .id("soloist-log")
                        .w_full()
                        .h(px(180.))
                        .overflow_y_scroll()
                        .p_3()
                        .rounded(theme::CONTROL_RADIUS)
                        .border_1()
                        .border_color(theme::hairline())
                        .bg(theme::window())
                        .text_size(px(11.))
                        .font_family("Consolas")
                        .text_color(theme::text_secondary())
                        .children(recent_log.into_iter().map(|line| div().child(line))),
                )
                .child({
                    let store = store.clone();

                    accent_button(
                        "retry",
                        "Try again",
                        Some(icon(IconName::RefreshCw, 15., theme::on_accent())),
                        true,
                    )
                        .on_click(move |_, _, cx| store.update(cx, |state, cx| state.retry(cx)))
                })
                .child(
                    div()
                        .id("change-key")
                        .cursor_pointer()
                        .tab_index(0)
                        .text_size(px(13.))
                        .text_color(theme::accent())
                        .hover(|style| style.text_color(theme::accent_hover()))
                        .focus_visible(|style| style.text_color(theme::text_primary()))
                        .child("Change Soloist key")
                        .on_click(move |_, _, cx: &mut App| {
                            store.update(cx, |state, cx| state.edit_key(cx))
                        }),
                )
                .into_any_element(),
            Status::Starting | Status::Stopped | Status::Ready { .. } => card()
                .child(title("Starting Soloist"))
                .child(spinner_row("One moment"))
                .into_any_element(),
        };

        div()
            .id("setup")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .p_8()
            .overflow_y_scroll()
            .child(content)
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
    let mut column = card()
        .child(logo_mark(64.))
        .child(title(heading))
        .child(body(text))
        .child(link(
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

            column.child(progress_bar(fraction)).child(note(label))
        }
        InstallState::Verifying => column
            .child(progress_bar(Some(1.)))
            .child(note("Checking that the download is signed by Spotify")),
        InstallState::Finalizing => column
            .child(progress_bar(Some(1.)))
            .child(note("Installing")),
        InstallState::Idle | InstallState::Failed(_) => column,
    };

    if let InstallState::Failed(reason) = install {
        column = column.child(
            div()
                .text_size(px(13.))
                .text_color(theme::warning())
                .child(format!("The download was not installed: {reason}")),
        );
    }

    if !busy {
        let label = if matches!(install, InstallState::Failed(_)) {
            "Try again"
        } else {
            button
        };

        column = column.child(
            accent_button(
                "download-soloist",
                label,
                Some(icon(IconName::Download, 15., theme::on_accent())),
                true,
            )
            .on_click(move |_, _, cx| store.update(cx, |state, cx| state.start_install(cx))),
        );
    }

    column.into_any_element()
}
