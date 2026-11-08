//! Every command the user can bind, the default keystrokes, and the pure
//! logic that merges the user's overrides into the effective bindings.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::{
    Action, App, KeyBinding, KeyBindingContextPredicate, Keystroke, NoAction, OperatingSystem,
    actions,
};
use rust_i18n::t;
use tracing::warn;

actions!(
    sonear,
    [
        OpenPalette,
        OpenShortcuts,
        OpenSettings,
        TogglePlayPause,
        NextTrack,
        PreviousTrack,
        SeekForward,
        SeekBack,
        VolumeUp,
        VolumeDown,
        ToggleShuffle,
        CycleRepeat,
        LikeCurrent,
        StartRadio,
        ToggleQueue,
        ToggleLyrics,
        ToggleMiniPlayer,
        ClearQueue,
        ToggleSidebar,
        GoHome,
        GoSearch,
        GoLibrary,
        GoLiked,
        GoHistory,
        GoBack,
        CloseOverlay,
    ]
);

/// Key context of the shell, the root every shortcut is resolved against.
pub const CONTEXT: &str = "Sonear";
/// Shortcuts that must not fire while a text field has focus.
const NOT_TYPING: &str = "Sonear && !Input";
const OS: OperatingSystem = OperatingSystem::Windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Playback,
    Queue,
    Navigation,
    General,
}

impl Group {
    pub const ALL: [Self; 4] = [Self::Playback, Self::Queue, Self::Navigation, Self::General];

    pub fn label(self) -> String {
        match self {
            Self::Playback => t!("group.playback"),
            Self::Queue => t!("group.queue"),
            Self::Navigation => t!("group.navigation"),
            Self::General => t!("group.general"),
        }
        .into_owned()
    }
}

pub struct Shortcut {
    pub id: &'static str,
    pub group: Group,
    pub default: Option<&'static str>,
    action: fn() -> Box<dyn Action>,
}

impl Shortcut {
    pub fn label(&self) -> String {
        let key = format!("shortcut.{}", self.id);

        t!(&key).into_owned()
    }

    pub fn action(&self) -> Box<dyn Action> {
        (self.action)()
    }

    /// The palette and the shortcut list stay reachable: unbinding either
    /// could lock the user out of the list that fixes it.
    pub fn locked(&self) -> bool {
        self.global()
    }

    fn global(&self) -> bool {
        matches!(self.id, "palette" | "shortcuts")
    }

    fn context(&self) -> &'static str {
        if self.global() { CONTEXT } else { NOT_TYPING }
    }
}

macro_rules! shortcut {
    ($id:literal, $group:ident, $default:expr, $action:ident) => {
        Shortcut {
            id: $id,
            group: Group::$group,
            default: $default,
            action: || Box::new($action),
        }
    };
}

pub static SHORTCUTS: [Shortcut; 25] = [
    shortcut!("palette", General, Some("ctrl-k"), OpenPalette),
    shortcut!("shortcuts", General, Some("ctrl-/"), OpenShortcuts),
    shortcut!(
        "toggle_mini_player",
        General,
        Some("ctrl-shift-m"),
        ToggleMiniPlayer
    ),
    shortcut!("settings", General, Some("ctrl-,"), OpenSettings),
    shortcut!("play_pause", Playback, Some("space"), TogglePlayPause),
    shortcut!("next", Playback, Some("ctrl-right"), NextTrack),
    shortcut!("previous", Playback, Some("ctrl-left"), PreviousTrack),
    shortcut!("seek_forward", Playback, Some("shift-right"), SeekForward),
    shortcut!("seek_back", Playback, Some("shift-left"), SeekBack),
    shortcut!("volume_up", Playback, Some("ctrl-up"), VolumeUp),
    shortcut!("volume_down", Playback, Some("ctrl-down"), VolumeDown),
    shortcut!("shuffle", Playback, Some("ctrl-s"), ToggleShuffle),
    shortcut!("repeat", Playback, Some("ctrl-r"), CycleRepeat),
    shortcut!("like", Playback, Some("alt-shift-b"), LikeCurrent),
    shortcut!("start_radio", Playback, None, StartRadio),
    shortcut!("toggle_queue", Queue, Some("alt-shift-q"), ToggleQueue),
    shortcut!("toggle_lyrics", Queue, Some("ctrl-shift-l"), ToggleLyrics),
    shortcut!("clear_queue", Queue, None, ClearQueue),
    shortcut!(
        "toggle_sidebar",
        Navigation,
        Some("alt-shift-l"),
        ToggleSidebar
    ),
    shortcut!("go_home", Navigation, None, GoHome),
    shortcut!("go_search", Navigation, Some("ctrl-l"), GoSearch),
    shortcut!("go_library", Navigation, None, GoLibrary),
    shortcut!("go_liked", Navigation, None, GoLiked),
    shortcut!("go_history", Navigation, None, GoHistory),
    shortcut!("back", Navigation, Some("alt-left"), GoBack),
];

pub type Binding = (&'static Shortcut, Option<String>);

pub fn find(id: &str) -> Option<&'static Shortcut> {
    SHORTCUTS.iter().find(|shortcut| shortcut.id == id)
}

/// Parses `source` and writes it back in one canonical spelling, so equal
/// keystrokes compare equal. `None` when GPUI rejects it.
pub fn normalize(source: &str) -> Option<String> {
    let source = source.trim();

    if source.contains(char::is_whitespace) {
        return None;
    }

    Keystroke::parse_with_os(source, OS)
        .ok()
        .filter(|keystroke| !keystroke.is_modifier_key())
        .map(|keystroke| keystroke.unparse(OS))
}

/// Writes `keystroke` in the spelling the config file uses.
pub fn spell(keystroke: &Keystroke) -> String {
    keystroke.unparse(OS)
}

/// The keystroke a config spelling stands for, to draw it as a key chip.
pub fn parse(source: &str) -> Option<Keystroke> {
    Keystroke::parse_with_os(source, OS).ok()
}

/// The keystroke each shortcut ends up with. An override replaces the default
/// and `""` means unbound. Unknown ids and keystrokes GPUI rejects are ignored.
/// When two shortcuts share a keystroke, an overridden one keeps it.
pub fn effective(overrides: &BTreeMap<String, String>) -> Vec<Binding> {
    for id in overrides.keys() {
        if find(id).is_none() {
            warn!(id, "ignoring a shortcut override with an unknown id");
        }
    }

    let mut bindings: Vec<(&'static Shortcut, Option<String>, bool)> = SHORTCUTS
        .iter()
        .map(|shortcut| {
            let default = shortcut.default.and_then(normalize);

            match overrides.get(shortcut.id).map(String::as_str) {
                None => (shortcut, default, false),
                Some("") if shortcut.locked() => (shortcut, default, false),
                Some("") => (shortcut, None, true),
                Some(source) => match normalize(source) {
                    Some(keystroke) => (shortcut, Some(keystroke), true),
                    None => {
                        warn!(
                            id = shortcut.id,
                            keystroke = source,
                            "ignoring an invalid shortcut"
                        );

                        (shortcut, default, false)
                    }
                },
            }
        })
        .collect();

    for index in 0..bindings.len() {
        let Some(keystroke) = bindings[index].1.clone() else {
            continue;
        };
        let owners: Vec<usize> = (0..bindings.len())
            .filter(|&other| bindings[other].1.as_deref() == Some(keystroke.as_str()))
            .collect();

        if owners.first() != Some(&index) {
            continue;
        }

        let winner = owners
            .iter()
            .copied()
            .find(|&owner| bindings[owner].0.locked())
            .or_else(|| owners.iter().copied().find(|&owner| bindings[owner].2))
            .unwrap_or(index);

        for owner in owners {
            if owner != winner {
                bindings[owner].1 = None;
            }
        }
    }

    bindings
        .into_iter()
        .map(|(shortcut, keystroke, _)| (shortcut, keystroke))
        .collect()
}

/// Stores `keystroke` for `id`, or drops the override when it equals the
/// default. Returns the new overrides and the shortcut that lost the
/// keystroke, which is unbound explicitly so it stays unbound.
pub fn assign(
    overrides: &BTreeMap<String, String>,
    id: &str,
    keystroke: &str,
) -> (BTreeMap<String, String>, Option<&'static str>) {
    let (Some(shortcut), Some(keystroke)) = (find(id), normalize(keystroke)) else {
        return (overrides.clone(), None);
    };
    let mut next = overrides.clone();
    let loser = effective(overrides)
        .into_iter()
        .find(|(other, current)| {
            other.id != shortcut.id && current.as_deref() == Some(keystroke.as_str())
        })
        .map(|(other, _)| other);

    if let Some(loser) = loser {
        if loser.locked() {
            return (overrides.clone(), None);
        }

        set_override(&mut next, loser, None);
    }

    set_override(&mut next, shortcut, Some(keystroke));

    (next, loser.map(|loser| loser.id))
}

/// The shortcut that can't be unbound and already uses `keystroke`, which
/// `assign` refuses to take it from.
pub fn locked_owner(
    overrides: &BTreeMap<String, String>,
    id: &str,
    keystroke: &str,
) -> Option<&'static Shortcut> {
    let keystroke = normalize(keystroke)?;

    effective(overrides)
        .into_iter()
        .find(|(other, current)| {
            other.id != id && other.locked() && current.as_deref() == Some(keystroke.as_str())
        })
        .map(|(other, _)| other)
}

/// Unbinds `id`.
pub fn unbind(overrides: &BTreeMap<String, String>, id: &str) -> BTreeMap<String, String> {
    let mut next = overrides.clone();

    if let Some(shortcut) = find(id) {
        set_override(&mut next, shortcut, None);
    }

    next
}

fn set_override(
    overrides: &mut BTreeMap<String, String>,
    shortcut: &Shortcut,
    keystroke: Option<String>,
) {
    let default = shortcut.default.and_then(normalize);

    if keystroke == default {
        overrides.remove(shortcut.id);
    } else {
        overrides.insert(shortcut.id.to_owned(), keystroke.unwrap_or_default());
    }
}

fn predicate(context: &str) -> Option<Rc<KeyBindingContextPredicate>> {
    match KeyBindingContextPredicate::parse(context) {
        Ok(predicate) => Some(Rc::new(predicate)),
        Err(error) => {
            warn!(%error, context, "invalid key context");

            None
        }
    }
}

fn binding(
    keystroke: &str,
    action: Box<dyn Action>,
    context: &str,
    cx: &App,
) -> Option<KeyBinding> {
    match KeyBinding::load(keystroke, action, predicate(context), false, None, cx) {
        Ok(binding) => Some(binding),
        Err(error) => {
            warn!(%error, keystroke, "could not bind a shortcut");

            None
        }
    }
}

/// Binds the fixed Escape that closes overlays. It sits at the shell's
/// depth, so the Escape of a palette or a text field wins when one has focus.
pub fn bind_fixed(cx: &mut App) {
    let bindings: Vec<KeyBinding> = binding("escape", Box::new(CloseOverlay), CONTEXT, cx)
        .into_iter()
        .collect();

    cx.bind_keys(bindings);
}

/// Writes the keymap. `previous` is what was bound before, empty at startup.
/// GPUI can't remove a binding, so each keystroke that is no longer bound to
/// its action gets a `NoAction` binding first, and the new bindings come after
/// it and win.
pub fn apply(cx: &mut App, previous: &[Binding], next: &[Binding]) {
    let mut bindings = Vec::new();

    for (shortcut, old) in previous {
        let Some(old) = old else {
            continue;
        };
        let kept = next
            .iter()
            .any(|(other, current)| other.id == shortcut.id && current.as_ref() == Some(old));

        if !kept {
            bindings.extend(binding(old, Box::new(NoAction), shortcut.context(), cx));
        }
    }

    for (shortcut, keystroke) in next {
        let Some(keystroke) = keystroke else {
            continue;
        };
        let unchanged = previous
            .iter()
            .any(|(other, old)| other.id == shortcut.id && old.as_ref() == Some(keystroke));

        if !unchanged {
            bindings.extend(binding(
                keystroke,
                shortcut.action(),
                shortcut.context(),
                cx,
            ));
        }
    }

    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn key_of(bindings: &[Binding], id: &str) -> Option<String> {
        bindings
            .iter()
            .find(|(shortcut, _)| shortcut.id == id)
            .and_then(|(_, keystroke)| keystroke.clone())
    }

    #[test]
    fn the_table_is_consistent() {
        let mut ids: Vec<_> = SHORTCUTS.iter().map(|shortcut| shortcut.id).collect();

        ids.sort_unstable();
        ids.dedup();

        assert_eq!(ids.len(), SHORTCUTS.len());

        let bindings = effective(&BTreeMap::new());

        for (shortcut, keystroke) in &bindings {
            assert_eq!(
                keystroke.is_some(),
                shortcut.default.is_some(),
                "{}",
                shortcut.id
            );
        }

        let mut keys: Vec<_> = bindings.iter().filter_map(|(_, key)| key.clone()).collect();
        let count = keys.len();

        keys.sort_unstable();
        keys.dedup();

        assert_eq!(keys.len(), count);
    }

    #[test]
    fn defaults_are_used_without_overrides() {
        let bindings = effective(&BTreeMap::new());

        assert_eq!(key_of(&bindings, "play_pause").as_deref(), Some("space"));
        assert_eq!(key_of(&bindings, "like").as_deref(), Some("alt-shift-b"));
        assert_eq!(key_of(&bindings, "clear_queue"), None);
    }

    #[test]
    fn overrides_replace_defaults_and_empty_unbinds() {
        let bindings = effective(&map(&[
            ("play_pause", "p"),
            ("next", ""),
            ("go_home", "ctrl-1"),
        ]));

        assert_eq!(key_of(&bindings, "play_pause").as_deref(), Some("p"));
        assert_eq!(key_of(&bindings, "next"), None);
        assert_eq!(key_of(&bindings, "go_home").as_deref(), Some("ctrl-1"));
    }

    #[test]
    fn unknown_ids_and_invalid_keystrokes_are_ignored() {
        let bindings = effective(&map(&[
            ("nope", "p"),
            ("play_pause", "ctrl-alt"),
            ("next", "a b"),
        ]));

        assert_eq!(bindings.len(), SHORTCUTS.len());
        assert_eq!(key_of(&bindings, "play_pause").as_deref(), Some("space"));
        assert_eq!(key_of(&bindings, "next").as_deref(), Some("ctrl-right"));
    }

    #[test]
    fn spellings_of_one_keystroke_are_equal() {
        let bindings = effective(&map(&[("go_home", "shift-alt-b")]));

        assert_eq!(key_of(&bindings, "go_home").as_deref(), Some("alt-shift-b"));
        assert_eq!(key_of(&bindings, "like"), None);
    }

    #[test]
    fn the_overridden_shortcut_keeps_a_shared_keystroke() {
        let bindings = effective(&map(&[("next", "space")]));

        assert_eq!(key_of(&bindings, "next").as_deref(), Some("space"));
        assert_eq!(key_of(&bindings, "play_pause"), None);
    }

    #[test]
    fn assign_stores_an_override_and_reports_the_loser() {
        let (overrides, loser) = assign(&BTreeMap::new(), "next", "space");

        assert_eq!(loser, Some("play_pause"));
        assert_eq!(overrides, map(&[("next", "space"), ("play_pause", "")]));

        let bindings = effective(&overrides);

        assert_eq!(key_of(&bindings, "next").as_deref(), Some("space"));
        assert_eq!(key_of(&bindings, "play_pause"), None);
    }

    #[test]
    fn assign_to_a_free_keystroke_loses_nothing() {
        let (overrides, loser) = assign(&BTreeMap::new(), "play_pause", "p");

        assert_eq!(loser, None);
        assert_eq!(overrides, map(&[("play_pause", "p")]));
    }

    #[test]
    fn assigning_the_default_back_removes_the_override() {
        let start = map(&[("play_pause", "p")]);
        let (overrides, loser) = assign(&start, "play_pause", "space");

        assert_eq!(loser, None);
        assert!(overrides.is_empty());
    }

    #[test]
    fn taking_a_default_back_from_its_thief_clears_both_overrides() {
        let (stolen, _) = assign(&BTreeMap::new(), "next", "space");
        let (restored, loser) = assign(&stolen, "play_pause", "space");

        assert_eq!(loser, Some("next"));
        assert_eq!(restored, map(&[("next", "")]));
    }

    #[test]
    fn unbind_stores_an_empty_override_unless_already_unbound_by_default() {
        assert_eq!(unbind(&BTreeMap::new(), "like"), map(&[("like", "")]));
        assert!(unbind(&BTreeMap::new(), "clear_queue").is_empty());
    }

    #[test]
    fn invalid_input_changes_nothing() {
        let start = map(&[("like", "")]);

        assert_eq!(assign(&start, "like", "ctrl-alt"), (start.clone(), None));
        assert_eq!(assign(&start, "missing", "p"), (start, None));
    }

    #[test]
    fn the_palette_and_list_keep_their_keys() {
        let (overrides, loser) = assign(&BTreeMap::new(), "next", "ctrl-k");

        assert!(overrides.is_empty());
        assert_eq!(loser, None);
        assert_eq!(
            locked_owner(&BTreeMap::new(), "next", "ctrl-k").map(|s| s.id),
            Some("palette")
        );

        let edited = BTreeMap::from([
            ("palette".to_owned(), String::new()),
            ("shortcuts".to_owned(), "ctrl-l".to_owned()),
        ]);
        let bindings = effective(&edited);
        let key = |id: &str| {
            bindings
                .iter()
                .find(|(shortcut, _)| shortcut.id == id)
                .and_then(|(_, keystroke)| keystroke.clone())
        };

        assert_eq!(key("palette").as_deref(), Some("ctrl-k"));
        assert_eq!(key("shortcuts").as_deref(), Some("ctrl-l"));
        assert_eq!(key("go_search"), None);
    }

    #[test]
    fn modifier_only_keystrokes_are_rejected() {
        assert_eq!(normalize("shift"), None);
        assert_eq!(normalize("ctrl-shift"), None);
    }
}
