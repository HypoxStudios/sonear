use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};

use crate::format::megabytes;
use crate::legal::LegalDoc;
use crate::protocol::{self, Registration};
use crate::store::{AppStore, Confirm, DEVICE_NAME_MAX, InstallState};
use crate::theme;
use crate::ui::widgets::{danger_button, ghost_button, icon, label_caps};

const RETENTION_CHOICES: [u32; 5] = [1, 3, 6, 12, 24];
const SPOTIFY_APPS_URL: &str = "https://www.spotify.com/account/apps/";
const SOURCE_URL: &str = "https://github.com/HypoxStudios/sonear";
const LICENSE_URL: &str = "https://github.com/HypoxStudios/sonear/blob/main/LICENSE";
const SUPPORT_EMAIL: &str = "support@hypox.studio";
const SUPPORT_MAILTO: &str = "mailto:support@hypox.studio";
const CLIENT_ID_SHOWN: usize = 6;

pub struct SettingsPage {
    store: Entity<AppStore>,
    device_input: Entity<InputState>,
    key_input: Entity<InputState>,
    key_open: bool,
    /// Read once and after each change: the page redraws on every playback event.
    links: Registration,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let device_name = store.read(cx).core.config.get().device_name;
        let device_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Device name");

            input.set_value(device_name, window, cx);

            input
        });
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Paste your Soloist key")
        });
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(
                &device_input,
                |page, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => page.save_device_name(cx),
                    _ => {}
                },
            ),
        ];

        Self {
            store,
            device_input,
            key_input,
            key_open: false,
            links: protocol::status(),
            _subscriptions: subscriptions,
        }
    }

    fn typed_device_name(&self, cx: &Context<Self>) -> String {
        self.device_input
            .read(cx)
            .value()
            .trim()
            .chars()
            .take(DEVICE_NAME_MAX)
            .collect()
    }

    fn can_save_device_name(&self, cx: &Context<Self>) -> bool {
        let typed = self.typed_device_name(cx);

        !typed.is_empty() && typed != self.store.read(cx).core.config.get().device_name
    }

    fn save_device_name(&mut self, cx: &mut Context<Self>) {
        if !self.can_save_device_name(cx) {
            return;
        }

        let name = self.typed_device_name(cx);

        self.store
            .update(cx, |state, cx| state.save_device_name(&name, cx));
        cx.notify();
    }

    fn open_key_editor(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |state, cx| {
            state.key_error = None;
            cx.notify();
        });
        self.key_open = true;
        cx.notify();
    }

    fn close_key_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key_open = false;
        self.key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key_input.read(cx).value().to_string();

        if key.trim().is_empty() {
            self.store.update(cx, |state, cx| {
                state.key_error = Some("Paste your Soloist key first.".to_owned());
                cx.notify();
            });

            return;
        }

        self.store.update(cx, |state, cx| state.save_key(key, cx));
        self.close_key_editor(window, cx);
    }

    fn register_links(&mut self, open_default_apps: bool, cx: &mut Context<Self>) {
        match protocol::register() {
            Ok(()) if open_default_apps => protocol::open_default_apps(cx),
            Ok(()) => {}
            Err(error) => self.report(error, cx),
        }

        self.links = protocol::status();
        cx.notify();
    }

    fn remove_links(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = protocol::unregister() {
            self.report(error, cx);
        }

        self.links = protocol::status();
        cx.notify();
    }

    fn report(&self, error: protocol::ProtocolError, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| {
            store.show_toast(
                format!("Could not change spotify: link handling: {error}"),
                cx,
            );
        });
    }

    fn links_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let controls = div().flex().items_center().gap_2();
        let controls = match self.links {
            Registration::NotRegistered => controls.child(
                ghost_button("links-setup", "Set up", true)
                    .on_click(cx.listener(|page, _, _, cx| page.register_links(true, cx))),
            ),
            Registration::Registered => controls
                .child(
                    ghost_button("links-choose", "Choose in Windows", true)
                        .on_click(cx.listener(|_, _, _, cx| protocol::open_default_apps(cx))),
                )
                .child(
                    ghost_button("links-remove", "Remove", true)
                        .on_click(cx.listener(|page, _, _, cx| page.remove_links(cx))),
                ),
            Registration::Outdated => controls.child(
                ghost_button("links-update", "Update", true)
                    .on_click(cx.listener(|page, _, _, cx| page.register_links(false, cx))),
            ),
        };

        row(
            "Spotify links",
            "Open spotify: links from your browser and other apps in Sonear. Windows asks you to pick Sonear under Default apps.",
            None,
            controls,
        )
    }

    fn lyrics_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            "Lyrics",
            "Shows lyrics from LRCLIB. While the panel is open, the song's title, artist, album and length are sent to lrclib.net.",
            None,
            Switch::new("settings-lyrics")
                .checked(self.store.read(cx).lyrics_enabled)
                .color(theme::accent())
                .accessibility_label("Lyrics")
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_lyrics_enabled(*enabled, cx));
                }),
        )
    }

    fn accent_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            "Accent from cover art",
            "Tints buttons and sliders with a colour from the playing song's cover.",
            None,
            Switch::new("settings-cover-accent")
                .checked(self.store.read(cx).cover_accent)
                .color(theme::accent())
                .accessibility_label("Accent from cover art")
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_cover_accent(*enabled, cx));
                }),
        )
    }

    fn tray_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            "Keep running in the tray",
            "Closing the window keeps Sonear and the music running. Quit from the tray icon.",
            None,
            Switch::new("settings-tray")
                .checked(self.store.read(cx).close_to_tray)
                .color(theme::accent())
                .accessibility_label("Keep running in the tray")
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_close_to_tray(*enabled, cx));
                }),
        )
    }

    fn playback_section(&self, cx: &mut Context<Self>) -> Div {
        let can_save = self.can_save_device_name(cx);
        let save = ghost_button("device-save", "Save", can_save).when(can_save, |this| {
            this.on_click(cx.listener(|page, _, _, cx| page.save_device_name(cx)))
        });

        let mut rows = vec![
            row(
                "Device name",
                "The name Spotify apps show for this computer.",
                None,
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(theme::SETTINGS_CONTROL_WIDTH)
                            .child(Input::new(&self.device_input)),
                    )
                    .child(save),
            ),
            row(
                "Crossfade",
                "Follows your Spotify account. Change it in the Spotify app under Settings \u{203a} Playback.",
                None,
                div(),
            ),
        ];

        if !self.store.read(cx).is_demo() {
            rows.push(self.links_row(cx));
        }

        rows.push(self.lyrics_row(cx));
        rows.push(self.accent_row(cx));
        rows.push(self.tray_row(cx));

        section("Playback", rows)
    }

    fn history_section(&self, cx: &Context<Self>) -> Div {
        let state = self.store.read(cx);
        let months = state.core.config.get().history_retention_months;
        let chips = div()
            .flex()
            .items_center()
            .gap_2()
            .children(RETENTION_CHOICES.iter().map(|choice| {
                let store = self.store.clone();
                let choice = *choice;

                chip(
                    ElementId::from((SharedString::from("retention"), choice as usize)),
                    choice.to_string(),
                    months == choice,
                )
                .on_click(move |_, _, cx| {
                    store.update(cx, |state, cx| state.set_history_retention(choice, cx));
                })
            }));
        let store = self.store.clone();

        section(
            "History",
            vec![
                row(
                    "Keep history for",
                    "Months. Older plays are deleted from this computer.",
                    None,
                    chips,
                ),
                row(
                    "Clear history",
                    "Deletes the list of plays Sonear keeps. Nothing changes in your Spotify account.",
                    None,
                    danger_button("settings-clear-history", "Clear history", true).on_click(
                        move |_, _, cx| {
                            store.update(cx, |state, cx| state.ask(Confirm::ClearHistory, cx));
                        },
                    ),
                ),
            ],
        )
    }

    fn keyboard_section(&self) -> Div {
        let store = self.store.clone();

        section(
            "Keyboard",
            vec![row(
                "Keyboard shortcuts",
                "See every shortcut and change the keys.",
                None,
                ghost_button("settings-shortcuts", "Open", true).on_click(move |_, _, cx| {
                    store.update(cx, |state, cx| state.open_shortcuts(cx));
                }),
            )],
        )
    }

    fn soloist_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.store.read(cx);
        let manager = state.core.soloist.clone()?;
        let version = manager.supervisor().version();
        let install = state.install.clone();
        let key_error = state.key_error.clone();
        let expiry_days = state.expiry_days;
        let busy = install.is_busy();
        let mut details = version.map_or_else(
            || "Version unknown".to_owned(),
            |version| format!("{} \u{00b7} built {}", version.version, version.build_date),
        );

        match expiry_days {
            Some(days) if days <= 0 => details.push_str(" \u{00b7} expired"),
            Some(1) => details.push_str(" \u{00b7} expires tomorrow"),
            Some(days) => details.push_str(&format!(" \u{00b7} expires in {days} days")),
            None => {}
        }

        let store = self.store.clone();
        let update = ghost_button(
            "settings-update",
            if busy { "Updating\u{2026}" } else { "Update" },
            !busy,
        )
        .when(!busy, |this| {
            this.on_click(move |_, _, cx| store.update(cx, |state, cx| state.start_install(cx)))
        });
        let key_control: AnyElement = if self.key_open {
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap_2()
                .child(
                    div()
                        .w(theme::SETTINGS_CONTROL_WIDTH)
                        .child(Input::new(&self.key_input).mask_toggle()),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            ghost_button("settings-key-cancel", "Cancel", true).on_click(
                                cx.listener(|page, _, window, cx| {
                                    page.close_key_editor(window, cx);
                                }),
                            ),
                        )
                        .child(ghost_button("settings-key-save", "Save", true).on_click(
                            cx.listener(|page, _, window, cx| page.save_key(window, cx)),
                        )),
                )
                .into_any_element()
        } else {
            ghost_button("settings-key-change", "Change key", true)
                .on_click(cx.listener(|page, _, _, cx| page.open_key_editor(cx)))
                .into_any_element()
        };

        Some(section(
            "Soloist",
            vec![
                row("Version", details, install_note(&install), update),
                row(
                    "Soloist key",
                    "Stored in Windows Credential Manager.",
                    key_error.map(|error| (error, theme::warning())),
                    key_control,
                ),
            ],
        ))
    }

    fn account_section(&self, cx: &Context<Self>) -> Div {
        let state = self.store.read(cx);

        if state.is_demo() {
            return section(
                "Spotify account",
                vec![row("Demo mode", "Nothing is stored.", None, div())],
            );
        }

        let name = state.profile.value().map_or_else(
            || "Spotify".to_owned(),
            |profile| profile.display_name.clone(),
        );
        let client_id = state.core.config.get().spotify_client_id;
        let shown: String = client_id.chars().take(CLIENT_ID_SHOWN).collect();
        let logout = self.store.clone();
        let delete = self.store.clone();

        section(
            "Spotify account",
            vec![
                row(
                    "Account",
                    format!("Signed in as {name}"),
                    None,
                    div()
                        .text_size(px(12.))
                        .text_color(theme::text_tertiary())
                        .child(format!("Client ID {shown}\u{2026}")),
                ),
                row(
                    "Log out",
                    "Removes your login, cached details, cover art, history and queue from this computer.",
                    None,
                    ghost_button("settings-logout", "Log out", true).on_click(move |_, _, cx| {
                        logout.update(cx, |state, cx| state.ask(Confirm::Logout, cx));
                    }),
                ),
                row(
                    "Delete all Sonear data",
                    "Also removes your Soloist key, pairing and settings, and starts setup again.",
                    None,
                    danger_button("settings-delete-all", "Delete all data", true).on_click(
                        move |_, _, cx| {
                            delete.update(cx, |state, cx| state.ask(Confirm::DeleteAll, cx));
                        },
                    ),
                ),
                row(
                    "Revoke access",
                    "Spotify keeps its own list of apps that can reach your account.",
                    None,
                    external_link(
                        "settings-revoke",
                        "Manage apps on Spotify",
                        SPOTIFY_APPS_URL,
                    ),
                ),
            ],
        )
    }

    fn about_section(&self) -> Div {
        let terms = self.store.clone();
        let privacy = self.store.clone();
        let links = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_6()
            .gap_y_2()
            .py_4()
            .child(
                text_link("about-terms", "End-user agreement").on_click(move |_, _, cx| {
                    terms.update(cx, |state, cx| state.open_legal(LegalDoc::Terms, cx));
                }),
            )
            .child(
                text_link("about-privacy", "Privacy policy").on_click(move |_, _, cx| {
                    privacy.update(cx, |state, cx| state.open_legal(LegalDoc::Privacy, cx));
                }),
            )
            .child(external_link(
                "about-contact",
                SUPPORT_EMAIL,
                SUPPORT_MAILTO,
            ))
            .child(external_link("about-source", "Source code", SOURCE_URL))
            .child(external_link(
                "about-license",
                "License: Apache-2.0",
                LICENSE_URL,
            ))
            .into_any_element();

        section(
            "About",
            vec![
                row(
                    format!("Sonear {}", env!("CARGO_PKG_VERSION")),
                    "Sonear is independent and isn't affiliated with or endorsed by Spotify.",
                    None,
                    div(),
                ),
                links,
            ],
        )
    }
}

fn install_note(install: &InstallState) -> Option<(String, Hsla)> {
    match install {
        InstallState::Idle => None,
        InstallState::Downloading { received, total } => Some((
            match total {
                Some(total) => format!(
                    "Downloading {} of {}",
                    megabytes(*received),
                    megabytes(*total)
                ),
                None => format!("Downloading {}", megabytes(*received)),
            },
            theme::text_tertiary(),
        )),
        InstallState::Verifying => Some((
            "Checking that the download is signed by Spotify".to_owned(),
            theme::text_tertiary(),
        )),
        InstallState::Finalizing => Some(("Installing".to_owned(), theme::text_tertiary())),
        InstallState::Failed(reason) => Some((
            format!("The download was not installed: {reason}"),
            theme::warning(),
        )),
    }
}

/// A heading and its rows, with a hairline between the rows.
fn section(title: &'static str, rows: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .pt_6()
        .child(div().pb_2().child(label_caps(title)))
        .children(rows.into_iter().enumerate().map(|(index, row)| {
            div()
                .when(index > 0, |this| {
                    this.border_t_1().border_color(theme::hairline())
                })
                .child(row)
        }))
}

fn row(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    note: Option<(String, Hsla)>,
    control: impl IntoElement,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .py_4()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text_primary())
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::text_secondary())
                        .child(description.into()),
                )
                .children(
                    note.map(|(text, color)| {
                        div().text_size(px(12.)).text_color(color).child(text)
                    }),
                ),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

fn chip(id: impl Into<ElementId>, label: String, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(32.))
        .min_w(px(40.))
        .px_3()
        .rounded(theme::CONTROL_RADIUS)
        .border_1()
        .border_color(theme::transparent())
        .cursor_pointer()
        .tab_index(0)
        .text_size(px(13.))
        .focus_visible(|style| style.border_color(theme::accent()))
        .map(|this| {
            if selected {
                this.bg(theme::active_nav())
                    .text_color(theme::text_primary())
                    .font_weight(FontWeight::MEDIUM)
            } else {
                this.text_color(theme::text_secondary())
                    .hover(|style| style.bg(theme::hover()))
            }
        })
        .child(label)
}

fn text_link(id: impl Into<ElementId>, text: &'static str) -> Stateful<Div> {
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
}

fn external_link(id: impl Into<ElementId>, text: &'static str, url: &'static str) -> AnyElement {
    text_link(id, text)
        .child(icon(IconName::ExternalLink, 12., theme::accent()))
        .on_click(move |_, _, cx: &mut App| cx.open_url(url))
        .into_any_element()
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let playback = self.playback_section(cx);
        let history = self.history_section(cx);
        let keyboard = self.keyboard_section();
        let soloist = self.soloist_section(cx);
        let account = self.account_section(cx);
        let about = self.about_section();

        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .px_6()
            .py_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(theme::SETTINGS_WIDTH)
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child("Settings"),
                    )
                    .child(playback)
                    .child(history)
                    .child(keyboard)
                    .children(soloist)
                    .child(account)
                    .child(about),
            )
    }
}
