use std::collections::BTreeMap;

use gpui_kit::component::kbd::Kbd;
use gpui_kit::{
    Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement, Keystroke, Modifiers,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, px, relative,
};

use crate::keymap::{self, Group};
use crate::store::AppStore;
use crate::theme;
use crate::ui::widgets::{ghost_button, label_caps};

pub struct ShortcutsView {
    store: Entity<AppStore>,
    recording: Option<&'static str>,
    note: Option<(&'static str, String)>,
    _interceptor: Option<Subscription>,
}

impl ShortcutsView {
    pub fn new(store: Entity<AppStore>) -> Self {
        Self {
            store,
            recording: None,
            note: None,
            _interceptor: None,
        }
    }

    fn overrides(&self, cx: &Context<Self>) -> BTreeMap<String, String> {
        self.store.read(cx).core.config.get().shortcuts
    }

    fn toggle_recording(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.note = None;

        if self.recording == Some(id) {
            self.stop_recording(cx);

            return;
        }

        let view = cx.entity().downgrade();

        self.recording = Some(id);
        self._interceptor = Some(cx.intercept_keystrokes(move |event, _, cx| {
            cx.stop_propagation();

            let keystroke = event.keystroke.clone();
            let _ = view.update(cx, |shortcuts, cx| shortcuts.capture(&keystroke, cx));
        }));
        cx.notify();
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        self._interceptor = None;
        cx.notify();
    }

    fn capture(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(id) = self.recording else {
            return;
        };

        if keystroke.is_modifier_key() {
            return;
        }

        let bare = keystroke.modifiers == Modifiers::default();

        match keystroke.key.as_str() {
            "escape" if bare => self.stop_recording(cx),
            "backspace" | "delete" if bare => self.unbind(id, cx),
            _ => self.assign(id, keystroke, cx),
        }
    }

    fn assign(&mut self, id: &'static str, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let spelled = keymap::spell(keystroke);

        if keymap::normalize(&spelled).is_none() {
            self.note = Some((id, "That key can't be used.".to_owned()));
            cx.notify();

            return;
        }

        let old = self.overrides(cx);

        if let Some(owner) = keymap::locked_owner(&old, id, &spelled) {
            self.note = Some((
                id,
                format!("Already used by {}, which always keeps a key.", owner.label),
            ));
            self.stop_recording(cx);

            return;
        }

        let (next, loser) = keymap::assign(&old, id, &spelled);

        self.note = loser
            .and_then(keymap::find)
            .map(|loser| (id, format!("Removed from {}", loser.label)));
        self.commit(&old, next, cx);
        self.stop_recording(cx);
    }

    fn unbind(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let Some(shortcut) = keymap::find(id) else {
            return;
        };

        if shortcut.locked() {
            self.note = Some((
                id,
                "This one can't be unbound, so you can always get back to this list.".to_owned(),
            ));
            self.stop_recording(cx);

            return;
        }

        let old = self.overrides(cx);
        let next = keymap::unbind(&old, id);

        self.commit(&old, next, cx);
        self.stop_recording(cx);
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        let old = self.overrides(cx);

        self.note = None;
        self.commit(&old, BTreeMap::new(), cx);
        self.stop_recording(cx);
    }

    fn commit(
        &mut self,
        old: &BTreeMap<String, String>,
        next: BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        if *old == next {
            return;
        }

        let previous = keymap::effective(old);
        let bindings = keymap::effective(&next);

        self.store
            .read(cx)
            .core
            .config
            .update(|config| config.shortcuts = next);
        keymap::apply(cx, &previous, &bindings);
        cx.notify();
    }

    fn chip(
        &self,
        id: &'static str,
        keystroke: Option<&str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let chip = div()
            .id(SharedString::from(format!("shortcut-{id}")))
            .flex()
            .items_center()
            .justify_end()
            .min_w(px(120.))
            .h(px(30.))
            .px_2()
            .rounded(theme::CONTROL_RADIUS)
            .border_1()
            .border_color(theme::transparent())
            .cursor_pointer()
            .tab_index(0)
            .hover(|style| style.bg(theme::hover()))
            .focus_visible(|style| style.border_color(theme::accent()))
            .on_click(cx.listener(move |shortcuts, _, _, cx| shortcuts.toggle_recording(id, cx)));

        if self.recording == Some(id) {
            return chip
                .bg(theme::recording())
                .border_color(theme::accent())
                .text_size(px(12.))
                .text_color(theme::accent())
                .child("Press keys\u{2026}");
        }

        match keystroke.and_then(keymap::parse) {
            Some(keystroke) => chip.child(Kbd::new(keystroke)),
            None => chip
                .text_size(px(12.))
                .text_color(theme::text_tertiary())
                .child("Not set"),
        }
    }

    fn row(
        &self,
        id: &'static str,
        label: &'static str,
        keystroke: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Div {
        let note = self
            .note
            .as_ref()
            .filter(|(noted, _)| *noted == id)
            .map(|(_, text)| text.clone());

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(theme::SHORTCUT_ROW_HEIGHT)
                    .text_size(px(13.))
                    .text_color(theme::text_primary())
                    .child(label)
                    .child(self.chip(id, keystroke, cx)),
            )
            .children(note.map(|text| {
                div()
                    .pb_2()
                    .text_size(px(12.))
                    .text_color(theme::warning())
                    .child(text)
            }))
    }
}

impl Render for ShortcutsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bindings = keymap::effective(&self.overrides(cx));
        let mut list = div().flex().flex_col().px_6().pb_2();

        for group in Group::ALL {
            list = list.child(div().pt_3().child(label_caps(group.label())));

            for (shortcut, keystroke) in bindings
                .iter()
                .filter(|(shortcut, _)| shortcut.group == group)
            {
                list = list.child(self.row(shortcut.id, shortcut.label, keystroke.as_deref(), cx));
            }
        }

        let close = self.store.clone();

        div()
            .id("shortcuts-backdrop")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .on_click(move |_, _, cx| {
                close.update(cx, |state, cx| state.close_shortcuts(cx));
            })
            .child(
                div()
                    .id("shortcuts-dialog")
                    .flex()
                    .flex_col()
                    .w(theme::OVERLAY_WIDTH)
                    .max_h(relative(theme::SHORTCUTS_MAX_HEIGHT))
                    .overflow_hidden()
                    .rounded(theme::PANEL_RADIUS)
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::panel())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex_none()
                            .px_6()
                            .pt_6()
                            .text_size(px(20.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child("Keyboard shortcuts"),
                    )
                    .child(
                        div()
                            .id("shortcuts-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(list),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .px_6()
                            .py_4()
                            .border_t_1()
                            .border_color(theme::hairline())
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::text_tertiary())
                                    .child(
                                        "Click a binding, then press the new keys. Esc cancels, Backspace unbinds.",
                                    ),
                            )
                            .child(
                                ghost_button("shortcuts-reset", "Reset to defaults", true)
                                    .flex_none()
                                    .on_click(cx.listener(|shortcuts, _, _, cx| shortcuts.reset(cx))),
                            ),
                    ),
            )
    }
}
