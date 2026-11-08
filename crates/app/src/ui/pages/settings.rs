use gpui_kit::assets::IconName;
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    div, px,
};
use rust_i18n::t;

use sonear_core::AppPaths;
use sonear_core::config::{Language, MINI_OPACITY_MAX, MINI_OPACITY_MIN};
use sonear_core::data_dir::resolve_target;

use crate::format::megabytes;
use crate::legal::LegalDoc;
use crate::protocol::{self, Registration};
use crate::site;
use crate::store::{
    AppStore, Confirm, DEVICE_NAME_MAX, InstallState, OutputChoice, UpdateState, open_logs_folder,
};
use crate::theme;
use crate::ui::widgets::{
    accent_button, danger_button, ghost_button, icon, label_caps, set_if_changed,
};

const RETENTION_CHOICES: [u32; 5] = [1, 3, 6, 12, 24];
const OPACITY_STEP: f32 = 5.;
const SPOTIFY_APPS_URL: &str = "https://www.spotify.com/account/apps/";
const SOURCE_URL: &str = "https://github.com/HypoxStudios/sonear";
const LICENSE_URL: &str = "https://github.com/HypoxStudios/sonear/blob/main/LICENSE";
const SUPPORT_EMAIL: &str = "support@hypox.studio";
const SUPPORT_MAILTO: &str = "mailto:support@hypox.studio";
const CLIENT_ID_SHOWN: usize = 6;

/// One entry of the output device dropdown; `None` is the Windows default.
#[derive(Clone)]
struct OutputItem {
    id: Option<String>,
    label: SharedString,
}

impl SelectItem for OutputItem {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Option<String> {
        &self.id
    }
}

fn output_items(choices: &[OutputChoice]) -> (Vec<OutputItem>, Option<IndexPath>) {
    let items = choices
        .iter()
        .map(|choice| OutputItem {
            id: choice.id.clone(),
            label: choice.label.clone().into(),
        })
        .collect();
    let selected = choices
        .iter()
        .position(|choice| choice.selected)
        .map(IndexPath::new);

    (items, selected)
}

pub struct SettingsPage {
    store: Entity<AppStore>,
    device_input: Entity<InputState>,
    output_select: Entity<SelectState<Vec<OutputItem>>>,
    /// What the dropdown shows, so it is only rebuilt when devices change.
    output_choices: Vec<OutputChoice>,
    key_input: Entity<InputState>,
    key_open: bool,
    opacity: Entity<SliderState>,
    dragging_opacity: bool,
    language_seen: u64,
    /// Read once and after each change: the page redraws on every playback event.
    links: Registration,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let device_name = store.read(cx).core.config.get().device_name;
        let device_input = cx.new(|cx| {
            let mut input =
                InputState::new(window, cx).placeholder(t!("settings.device.placeholder"));

            input.set_value(device_name, window, cx);

            input
        });
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("common.key_placeholder"))
        });
        let output_choices = store.read(cx).output_choices();
        let output_select = cx.new(|cx| {
            let (items, selected) = output_items(&output_choices);

            SelectState::new(items, selected, window, cx)
        });
        let language_seen = store.read(cx).language_version;
        let opacity = cx.new(|_| {
            SliderState::new()
                .min(f32::from(MINI_OPACITY_MIN))
                .max(f32::from(MINI_OPACITY_MAX))
                .step(OPACITY_STEP)
        });

        set_if_changed(
            &opacity,
            f32::from(store.read(cx).mini_player_opacity),
            0.5,
            window,
            cx,
        );

        let subscriptions = vec![
            cx.observe_in(&store, window, |page, _, window, cx| {
                page.sync_output_choices(window, cx);
                page.sync_opacity(window, cx);
                cx.notify();
            }),
            cx.subscribe(
                &output_select,
                |page, _, event: &SelectEvent<Vec<OutputItem>>, cx| {
                    let SelectEvent::Confirm(Some(device)) = event else {
                        return;
                    };

                    page.choose_output(device.clone(), cx);
                },
            ),
            cx.subscribe(&opacity, |page, _, event: &SliderEvent, cx| match event {
                SliderEvent::Change(value) => {
                    page.dragging_opacity = true;
                    page.store.update(cx, |store, cx| {
                        store.preview_mini_player_opacity(value.end().round() as u8, cx);
                    });
                }
                SliderEvent::Release(value) => {
                    page.dragging_opacity = false;
                    page.store.update(cx, |store, cx| {
                        store.set_mini_player_opacity(value.end().round() as u8, cx);
                    });
                }
            }),
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
            output_select,
            output_choices,
            key_input,
            key_open: false,
            opacity,
            dragging_opacity: false,
            language_seen,
            links: protocol::status(),
            _subscriptions: subscriptions,
        }
    }

    fn sync_opacity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dragging_opacity {
            return;
        }

        let target = f32::from(self.store.read(cx).mini_player_opacity);

        set_if_changed(&self.opacity, target, 0.5, window, cx);
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
                state.key_error = Some(t!("settings.soloist.key.empty").into_owned());
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
                t!("settings.links.failed", error = error.user_message()).into_owned(),
                cx,
            );
        });
    }

    fn links_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let controls = div().flex().items_center().gap_2();
        let controls = match self.links {
            Registration::NotRegistered => controls.child(
                ghost_button("links-setup", t!("settings.links.set_up"), true)
                    .on_click(cx.listener(|page, _, _, cx| page.register_links(true, cx))),
            ),
            Registration::Registered => controls
                .child(
                    ghost_button("links-choose", t!("settings.links.choose"), true)
                        .on_click(cx.listener(|_, _, _, cx| protocol::open_default_apps(cx))),
                )
                .child(
                    ghost_button("links-remove", t!("settings.links.remove"), true)
                        .on_click(cx.listener(|page, _, _, cx| page.remove_links(cx))),
                ),
            Registration::Outdated => controls.child(
                ghost_button("links-update", t!("settings.links.update"), true)
                    .on_click(cx.listener(|page, _, _, cx| page.register_links(false, cx))),
            ),
        };

        row(
            t!("settings.links.title"),
            t!("settings.links.body"),
            None,
            controls,
        )
    }

    /// Rebuilds the dropdown when devices come and go or the saved one changes.
    fn sync_output_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.store.read(cx).output_choices();

        if choices == self.output_choices {
            return;
        }

        let (items, selected) = output_items(&choices);

        self.output_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_index(selected, window, cx);
        });
        self.output_choices = choices;
    }

    fn choose_output(&mut self, device: Option<String>, cx: &mut Context<Self>) {
        let saved = self.store.read(cx).core.config.get().output_device;

        if saved == device {
            return;
        }

        self.store
            .update(cx, |state, cx| state.switch_output(device, cx));
    }

    fn output_row(&self, cx: &Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let available = state.output_routing_available();
        let select = div().w(theme::SETTINGS_CONTROL_WIDTH).child(
            Select::new(&self.output_select)
                .id("output-device")
                .disabled(!available || state.output_switching()),
        );

        row(
            t!("settings.output.title"),
            t!("settings.output.body"),
            (!available).then(|| {
                (
                    t!("settings.output.unavailable").into_owned(),
                    theme::text_tertiary(),
                )
            }),
            select,
        )
    }

    fn lyrics_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            t!("settings.lyrics.title"),
            t!("settings.lyrics.body"),
            None,
            Switch::new("settings-lyrics")
                .checked(self.store.read(cx).lyrics_enabled)
                .color(theme::accent())
                .accessibility_label(t!("settings.lyrics.title"))
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_lyrics_enabled(*enabled, cx));
                }),
        )
    }

    fn accent_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            t!("settings.accent.title"),
            t!("settings.accent.body"),
            None,
            Switch::new("settings-cover-accent")
                .checked(self.store.read(cx).cover_accent)
                .color(theme::accent())
                .accessibility_label(t!("settings.accent.title"))
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_cover_accent(*enabled, cx));
                }),
        )
    }

    fn tray_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            t!("settings.tray.title"),
            t!("settings.tray.body"),
            None,
            Switch::new("settings-tray")
                .checked(self.store.read(cx).close_to_tray)
                .color(theme::accent())
                .accessibility_label(t!("settings.tray.title"))
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_close_to_tray(*enabled, cx));
                }),
        )
    }

    fn minimize_tray_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            t!("settings.minimize_tray.title"),
            t!("settings.minimize_tray.body"),
            None,
            Switch::new("settings-minimize-tray")
                .checked(self.store.read(cx).minimize_to_tray)
                .color(theme::accent())
                .accessibility_label(t!("settings.minimize_tray.title"))
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_minimize_to_tray(*enabled, cx));
                }),
        )
    }

    fn mini_opacity_row(&self, cx: &Context<Self>) -> AnyElement {
        let percent = self.store.read(cx).mini_player_opacity;

        row(
            t!("settings.mini_opacity.title"),
            t!("settings.mini_opacity.body"),
            None,
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .w(theme::SETTINGS_CONTROL_WIDTH)
                        .child(Slider::new(&self.opacity)),
                )
                .child(
                    div()
                        .w(px(40.))
                        .text_size(px(12.))
                        .text_color(theme::text_secondary())
                        .child(format!("{percent}%")),
                ),
        )
    }

    fn detailed_logs_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            t!("settings.detailed_logs.title"),
            t!("settings.detailed_logs.body"),
            None,
            Switch::new("settings-detailed-logs")
                .checked(self.store.read(cx).core.config.get().detailed_logs)
                .color(theme::accent())
                .accessibility_label(t!("settings.detailed_logs.title"))
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| {
                        state
                            .core
                            .config
                            .update(|config| config.detailed_logs = *enabled);
                        cx.notify();
                    });
                }),
        )
    }

    fn general_section(&self, cx: &Context<Self>) -> Div {
        let current = self.store.read(cx).language();
        let chips =
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(Language::ALL.iter().enumerate().map(|(index, language)| {
                    let store = self.store.clone();
                    let language = *language;

                    chip(
                        ElementId::from((SharedString::from("language"), index)),
                        language.native_name().to_owned(),
                        current == language,
                    )
                    .on_click(move |_, _, cx| {
                        store.update(cx, |state, cx| state.set_language(language, cx));
                    })
                }));

        section(
            t!("settings.section.general"),
            vec![row(
                t!("settings.language.title"),
                t!("settings.language.body"),
                None,
                chips,
            )],
        )
    }

    fn playback_section(&self, cx: &mut Context<Self>) -> Div {
        let can_save = self.can_save_device_name(cx);
        let save = ghost_button("device-save", t!("settings.save"), can_save)
            .when(can_save, |this| {
                this.on_click(cx.listener(|page, _, _, cx| page.save_device_name(cx)))
            });

        let mut rows = vec![
            row(
                t!("settings.device.title"),
                t!("settings.device.body"),
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
                t!("settings.crossfade.title"),
                t!("settings.crossfade.body"),
                None,
                div(),
            ),
        ];

        rows.insert(1, self.output_row(cx));

        if !self.store.read(cx).is_demo() {
            rows.push(self.links_row(cx));
        }

        rows.push(self.lyrics_row(cx));
        rows.push(self.accent_row(cx));
        rows.push(self.tray_row(cx));
        rows.push(self.minimize_tray_row(cx));
        rows.push(self.mini_opacity_row(cx));

        section(t!("settings.section.playback"), rows)
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
                    retention_label(choice),
                    months == choice,
                )
                .on_click(move |_, _, cx| {
                    store.update(cx, |state, cx| state.set_history_retention(choice, cx));
                })
            }));
        let store = self.store.clone();

        section(
            t!("settings.section.history"),
            vec![
                row(
                    t!("settings.history.keep.title"),
                    t!("settings.history.keep.body"),
                    None,
                    chips,
                ),
                row(
                    t!("settings.history.clear.title"),
                    t!("settings.history.clear.body"),
                    None,
                    danger_button("settings-clear-history", t!("history.clear"), true).on_click(
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
            t!("settings.section.keyboard"),
            vec![row(
                t!("settings.keyboard.title"),
                t!("settings.keyboard.body"),
                None,
                ghost_button("settings-shortcuts", t!("settings.open"), true).on_click(
                    move |_, _, cx| {
                        store.update(cx, |state, cx| state.open_shortcuts(cx));
                    },
                ),
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
            || t!("settings.soloist.version_unknown").into_owned(),
            |version| {
                t!(
                    "settings.soloist.built",
                    version = version.version,
                    date = version.build_date
                )
                .into_owned()
            },
        );

        match expiry_days {
            Some(days) if days <= 0 => {
                details.push_str(&format!(" \u{00b7} {}", t!("settings.soloist.expired")));
            }
            Some(1) => {
                details.push_str(&format!(
                    " \u{00b7} {}",
                    t!("settings.soloist.expires_tomorrow")
                ));
            }
            Some(days) => {
                details.push_str(&format!(
                    " \u{00b7} {}",
                    t!("settings.soloist.expires_in", days = days)
                ));
            }
            None => {}
        }

        let store = self.store.clone();
        let update = ghost_button(
            "settings-update",
            if busy {
                t!("settings.soloist.updating")
            } else {
                t!("settings.soloist.update")
            },
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
                            ghost_button("settings-key-cancel", t!("common.cancel"), true)
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.close_key_editor(window, cx);
                                })),
                        )
                        .child(
                            ghost_button("settings-key-save", t!("settings.save"), true).on_click(
                                cx.listener(|page, _, window, cx| page.save_key(window, cx)),
                            ),
                        ),
                )
                .into_any_element()
        } else {
            ghost_button(
                "settings-key-change",
                t!("settings.soloist.key.change"),
                true,
            )
            .on_click(cx.listener(|page, _, _, cx| page.open_key_editor(cx)))
            .into_any_element()
        };

        Some(section(
            t!("settings.section.soloist"),
            vec![
                row(
                    t!("settings.soloist.version"),
                    details,
                    install_note(&install),
                    update,
                ),
                row(
                    t!("settings.soloist.key.title"),
                    t!("settings.soloist.key.body"),
                    key_error.map(|error| (error, theme::warning())),
                    key_control,
                ),
            ],
        ))
    }

    /// Asks for a folder, then asks to move the data into it.
    fn choose_data_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        let store = self.store.clone();

        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };

            store.update(cx, |store, cx| {
                store.propose_data_move(resolve_target(&folder), cx);
            });
        })
        .detach();
    }

    fn storage_section(&self, cx: &mut Context<Self>) -> Div {
        let state = self.store.read(cx);
        let demo = state.is_demo();
        let current = state.core.paths.local_root();
        let shown = if demo {
            AppPaths::default_local_root().unwrap_or_else(|| current.clone())
        } else {
            current.clone()
        };
        let custom = state.core.config.get().data_dir.is_some();
        let store = self.store.clone();
        let open = ghost_button("storage-open", t!("settings.open"), true)
            .on_click(move |_, _, cx| cx.open_with_system(&current));
        let controls = div()
            .flex()
            .items_center()
            .gap_2()
            .child(open)
            .child(
                ghost_button("storage-change", t!("settings.storage.change"), true)
                    .on_click(cx.listener(|page, _, _, cx| page.choose_data_folder(cx))),
            )
            .when(custom && !demo, |this| {
                this.child(
                    ghost_button("storage-default", t!("settings.storage.default"), true).on_click(
                        move |_, _, cx| {
                            store.update(cx, |store, cx| store.propose_default_data_move(cx));
                        },
                    ),
                )
            });

        let logs = ghost_button("logs-open", t!("settings.logs.open"), true)
            .on_click(|_, _, cx| open_logs_folder(cx));

        section(
            t!("settings.section.storage"),
            vec![
                row(
                    t!("settings.storage.title"),
                    t!("settings.storage.body"),
                    Some((shown.display().to_string(), theme::text_tertiary())),
                    controls,
                ),
                row(
                    t!("settings.logs.title"),
                    t!("settings.logs.body"),
                    None,
                    logs,
                ),
                self.detailed_logs_row(cx),
            ],
        )
    }

    fn account_section(&self, cx: &Context<Self>) -> Div {
        let state = self.store.read(cx);

        if state.is_demo() {
            return section(
                t!("settings.section.account"),
                vec![row(
                    t!("settings.account.demo_title"),
                    t!("settings.account.demo_body"),
                    None,
                    div(),
                )],
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
            t!("settings.section.account"),
            vec![
                row(
                    t!("settings.account.title"),
                    t!("settings.account.signed_in", name = name),
                    None,
                    div()
                        .text_size(px(12.))
                        .text_color(theme::text_tertiary())
                        .child(t!("settings.account.client_id", id = shown)),
                ),
                row(
                    t!("common.log_out"),
                    t!("settings.account.logout_body"),
                    None,
                    ghost_button("settings-logout", t!("common.log_out"), true).on_click(
                        move |_, _, cx| {
                            logout.update(cx, |state, cx| state.ask(Confirm::Logout, cx));
                        },
                    ),
                ),
                row(
                    t!("settings.account.delete.title"),
                    t!("settings.account.delete.body"),
                    None,
                    danger_button(
                        "settings-delete-all",
                        t!("settings.account.delete.button"),
                        true,
                    )
                    .on_click(move |_, _, cx| {
                        delete.update(cx, |state, cx| state.ask(Confirm::DeleteAll, cx));
                    }),
                ),
                row(
                    t!("settings.account.revoke.title"),
                    t!("settings.account.revoke.body"),
                    None,
                    external_link(
                        "settings-revoke",
                        t!("settings.account.revoke.link"),
                        SPOTIFY_APPS_URL,
                    ),
                ),
            ],
        )
    }

    fn updates_row(&self, cx: &Context<Self>) -> AnyElement {
        let store = self.store.clone();

        row(
            t!("settings.about.updates.title"),
            t!("settings.about.updates.body"),
            None,
            Switch::new("settings-updates")
                .checked(self.store.read(cx).check_for_updates)
                .color(theme::accent())
                .accessibility_label(t!("settings.about.updates.title"))
                .on_change(move |enabled, _, cx| {
                    store.update(cx, |state, cx| state.set_check_for_updates(*enabled, cx));
                }),
        )
    }

    fn version_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = format!("Sonear {}", env!("CARGO_PKG_VERSION"));
        let description = t!("settings.about.independent");
        let UpdateState::Ready { version, .. } = self.store.read(cx).update.clone() else {
            return row(title, description, None, div());
        };
        let store = self.store.clone();
        let controls = div()
            .flex()
            .items_center()
            .gap_4()
            .child(external_link(
                "about-whats-new",
                t!("settings.about.whats_new"),
                site::changelog(),
            ))
            .child(
                accent_button("about-restart", t!("sidebar.restart_to_update"), None, true)
                    .on_click(move |_, _, cx| {
                        store.update(cx, |state, cx| state.restart_to_update(cx));
                    }),
            );

        row(
            title,
            description,
            Some((
                t!("settings.about.ready", version = version).into_owned(),
                theme::accent(),
            )),
            controls,
        )
    }

    fn about_section(&self, cx: &mut Context<Self>) -> Div {
        let terms = self.store.clone();
        let privacy = self.store.clone();
        let links = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_6()
            .gap_y_2()
            .py_4()
            .child(external_link(
                "about-website",
                t!("settings.about.website"),
                site::home(),
            ))
            .child(
                text_link("about-terms", LegalDoc::Terms.title()).on_click(move |_, _, cx| {
                    terms.update(cx, |state, cx| state.open_legal(LegalDoc::Terms, cx));
                }),
            )
            .child(
                text_link("about-privacy", LegalDoc::Privacy.title()).on_click(move |_, _, cx| {
                    privacy.update(cx, |state, cx| state.open_legal(LegalDoc::Privacy, cx));
                }),
            )
            .child(external_link(
                "about-contact",
                SUPPORT_EMAIL,
                SUPPORT_MAILTO,
            ))
            .child(external_link(
                "about-source",
                t!("settings.about.source"),
                SOURCE_URL,
            ))
            .child(external_link(
                "about-license",
                t!("settings.about.license"),
                LICENSE_URL,
            ))
            .into_any_element();

        section(
            t!("settings.section.about"),
            vec![self.updates_row(cx), self.version_row(cx), links],
        )
    }
}

fn install_note(install: &InstallState) -> Option<(String, Hsla)> {
    match install {
        InstallState::Idle => None,
        InstallState::Downloading { received, total } => Some((
            match total {
                Some(total) => t!(
                    "install.downloading_of",
                    received = megabytes(*received),
                    total = megabytes(*total)
                )
                .into_owned(),
                None => t!("install.downloading", received = megabytes(*received)).into_owned(),
            },
            theme::text_tertiary(),
        )),
        InstallState::Verifying => {
            Some((t!("install.verifying").into_owned(), theme::text_tertiary()))
        }
        InstallState::Finalizing => Some((
            t!("install.installing").into_owned(),
            theme::text_tertiary(),
        )),
        InstallState::Failed(reason) => Some((
            t!("install.failed", reason = reason).into_owned(),
            theme::warning(),
        )),
    }
}

/// A heading and its rows, with a hairline between the rows.
fn section(title: impl Into<SharedString>, rows: Vec<AnyElement>) -> Div {
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

fn retention_label(months: u32) -> String {
    match months {
        1 => t!("settings.history.keep.choice_1"),
        3 => t!("settings.history.keep.choice_3"),
        6 => t!("settings.history.keep.choice_6"),
        12 => t!("settings.history.keep.choice_12"),
        24 => t!("settings.history.keep.choice_24"),
        other => return other.to_string(),
    }
    .into_owned()
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

fn text_link(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
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
}

fn external_link(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    url: impl Into<String>,
) -> AnyElement {
    let url = url.into();

    text_link(id, text)
        .child(icon(IconName::ExternalLink, 12., theme::accent()))
        .on_click(move |_, _, cx: &mut App| cx.open_url(&url))
        .into_any_element()
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.store.read(cx).language_version;

        if language != self.language_seen {
            self.language_seen = language;
            self.device_input.update(cx, |input, cx| {
                input.set_placeholder(t!("settings.device.placeholder"), window, cx);
            });
            self.key_input.update(cx, |input, cx| {
                input.set_placeholder(t!("common.key_placeholder"), window, cx);
            });
        }

        let general = self.general_section(cx);
        let playback = self.playback_section(cx);
        let history = self.history_section(cx);
        let keyboard = self.keyboard_section();
        let soloist = self.soloist_section(cx);
        let storage = self.storage_section(cx);
        let account = self.account_section(cx);
        let about = self.about_section(cx);

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
                            .child(t!("route.settings")),
                    )
                    .child(general)
                    .child(playback)
                    .child(history)
                    .child(keyboard)
                    .children(soloist)
                    .child(storage)
                    .child(account)
                    .child(about),
            )
    }
}
