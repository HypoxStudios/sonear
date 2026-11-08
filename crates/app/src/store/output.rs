//! Choosing where Soloist plays. Soloist picks its output device when it
//! starts and Windows can move it without telling anyone, so Sonear pins it
//! to the chosen device with a per-app setting and restarts it to apply one.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, Context};
use rust_i18n::t;
use sonear_core::model::PlayOffset;
use sonear_core::{AppEvent, PlaybackModel, Player, SoloistManager};
use sonear_soloist::Status;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout_at;
use tracing::{debug, warn};

use super::AppStore;
use crate::audio::{AudioError, AudioEvent, AudioEvents, AudioHandle, OutputDevice};
use crate::ui::output_popup;

/// A device that just connected isn't offered again for this long, because
/// Bluetooth devices can drop and come back within seconds.
const OFFER_AGAIN_AFTER: Duration = Duration::from_secs(10);
/// Windows moves Soloist as the device vanishes, so for this long the device
/// it left still counts as the one that disconnected.
const MOVED_AWAY_FOR: Duration = Duration::from_secs(3);
/// How long after a restart Sonear waits to resume playback.
const RESUME_WAIT: Duration = Duration::from_secs(15);
/// Soloist normally reports its last track right away; this is how long to
/// wait for it before playing without it.
const STATE_GRACE: Duration = Duration::from_secs(3);
/// Positions this close to the start aren't worth a seek.
const SEEK_MIN_MS: u64 = 1_000;

#[derive(Default)]
pub struct OutputState {
    audio: Option<AudioHandle>,
    pub routing_available: bool,
    pub devices: Vec<OutputDevice>,
    pub default_device: Option<String>,
    soloist_device: Option<String>,
    moved_from: Option<(String, Instant)>,
    /// The device whose loss paused playback, so plugging it back in can offer it.
    paused_by_disconnect: Option<String>,
    /// Whether the paused state of that pause has been seen, so a late
    /// "playing" report from before it doesn't clear it.
    disconnect_pause_seen: bool,
    popup: Option<AnyWindowHandle>,
    recently_offered: HashMap<String, Instant>,
    checked_pid: Option<u32>,
    /// A restart to fix the Windows setting just happened, so another one waits
    /// for a check that finds it right.
    corrected: bool,
    pub switching: bool,
}

/// One entry of the settings list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputChoice {
    pub id: Option<String>,
    pub label: String,
    pub selected: bool,
}

pub fn same_device(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// What the popup decision looks at.
pub struct OfferInput<'a> {
    pub routing: bool,
    pub playing: bool,
    pub paused_by_disconnect: bool,
    pub soloist_device: Option<&'a str>,
    /// How long ago this device was last offered.
    pub offered_ago: Option<Duration>,
}

/// Whether a device that just connected is worth a popup.
pub fn should_offer(device_id: &str, input: &OfferInput) -> bool {
    input.routing
        && (input.playing || input.paused_by_disconnect)
        && !same_device(input.soloist_device, Some(device_id))
        && input.offered_ago.is_none_or(|ago| ago >= OFFER_AGAIN_AFTER)
}

/// Whether losing `gone` takes the sound Soloist is playing away. Soloist
/// may already have been moved to the default device when the loss is noticed,
/// so the device it just left counts too.
pub fn loses_soloist(
    gone: &str,
    current: Option<&str>,
    moved_from: Option<(&str, Duration)>,
    playing: bool,
) -> bool {
    playing
        && (same_device(current, Some(gone))
            || moved_from.is_some_and(|(id, ago)| {
                ago <= MOVED_AWAY_FOR && same_device(Some(id), Some(gone))
            }))
}

/// How playback restarts after Soloist does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeCall<'a> {
    InContext { context: &'a str, track: &'a str },
    Track(&'a str),
}

/// A track inside an album, playlist or artist keeps its place in it; a track
/// on its own, or one whose context is just itself, plays alone.
pub fn resume_call<'a>(track: &'a str, context: Option<&'a str>) -> ResumeCall<'a> {
    match context {
        Some(context)
            if !context.is_empty()
                && !context.starts_with("spotify:track:")
                && !context.starts_with("spotify:episode:") =>
        {
            ResumeCall::InContext { context, track }
        }
        _ => ResumeCall::Track(track),
    }
}

/// Where playback was when a switch began.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumePlan {
    track: String,
    context: Option<String>,
    position_ms: u64,
}

impl ResumePlan {
    fn capture(model: &PlaybackModel, position_ms: u64) -> Option<Self> {
        Some(Self {
            track: model.item.as_ref()?.uri.clone(),
            context: model.context.as_ref().map(|context| context.uri.clone()),
            position_ms,
        })
    }
}

enum Check {
    Matches,
    Mismatch,
    Failed,
}

impl AppStore {
    pub fn output_routing_available(&self) -> bool {
        self.output.routing_available
    }

    pub fn output_switching(&self) -> bool {
        self.output.switching
    }

    /// The rows of the settings choice: the Windows default, each connected
    /// device, and the saved one when it isn't connected.
    pub fn output_choices(&self) -> Vec<OutputChoice> {
        let config = self.core.config.get();
        let saved = config.output_device.as_deref();
        let mut choices = vec![OutputChoice {
            id: None,
            label: t!("settings.output.default").into_owned(),
            selected: saved.is_none(),
        }];

        for device in &self.output.devices {
            choices.push(OutputChoice {
                id: Some(device.id.clone()),
                label: device.name.clone(),
                selected: same_device(saved, Some(&device.id)),
            });
        }

        if let Some(id) = saved
            && !self
                .output
                .devices
                .iter()
                .any(|device| same_device(Some(&device.id), Some(id)))
        {
            let name = config
                .output_device_name
                .clone()
                .unwrap_or_else(|| id.to_owned());

            choices.push(OutputChoice {
                id: Some(id.to_owned()),
                label: t!("settings.output.missing", name = name).into_owned(),
                selected: true,
            });
        }

        choices
    }

    /// Starts listening to the audio thread. Demo builds get two made-up devices.
    pub fn attach_audio(
        &mut self,
        audio: Option<(AudioHandle, AudioEvents)>,
        cx: &mut Context<Self>,
    ) {
        if self.is_demo() {
            self.attach_demo_devices(cx);

            return;
        }

        let Some((handle, mut events)) = audio else {
            return;
        };

        self.output.audio = Some(handle);

        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |store, cx| store.apply_audio(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn attach_demo_devices(&mut self, cx: &mut Context<Self>) {
        let speakers = OutputDevice {
            id: "demo-speakers".into(),
            name: "Speakers".into(),
        };
        let headphones = OutputDevice {
            id: "demo-headphones".into(),
            name: "Headphones".into(),
        };

        self.output.routing_available = true;
        self.output.default_device = Some(speakers.id.clone());
        self.output.soloist_device = Some(speakers.id.clone());
        self.output.devices = vec![speakers, headphones.clone()];

        #[cfg(feature = "demo")]
        if std::env::var("SONEAR_DEMO_OUTPUT_POPUP").is_ok() {
            self.show_output_popup(headphones, cx);
        }

        #[cfg(not(feature = "demo"))]
        let _ = (headphones, cx);
    }

    fn apply_audio(&mut self, event: AudioEvent, cx: &mut Context<Self>) {
        match event {
            AudioEvent::Routing(available) => self.output.routing_available = available,
            AudioEvent::Devices { devices, default } => {
                self.output.devices = devices;
                self.output.default_device = default;
                self.refresh_saved_device_name();
            }
            AudioEvent::Connected(device) => self.device_connected(device, cx),
            AudioEvent::Disconnected { id, name } => self.device_disconnected(&id, &name, cx),
            AudioEvent::SoloistDevice(id) => {
                debug!(device = %id, "Soloist is playing on this output device");

                let previous = self.output.soloist_device.replace(id.clone());

                if let Some(previous) =
                    previous.filter(|previous| !same_device(Some(previous), Some(&id)))
                {
                    self.output.moved_from = Some((previous, Instant::now()));
                }

                self.check_output_policy(cx);
            }
        }

        cx.notify();
    }

    fn refresh_saved_device_name(&self) {
        let config = self.core.config.get();
        let Some(saved) = config.output_device.as_deref() else {
            return;
        };
        let Some(device) = self
            .output
            .devices
            .iter()
            .find(|device| same_device(Some(&device.id), Some(saved)))
        else {
            return;
        };

        if config.output_device_name.as_deref() != Some(device.name.as_str()) {
            self.core
                .config
                .update(|config| config.output_device_name = Some(device.name.clone()));
        }
    }

    fn device_connected(&mut self, device: OutputDevice, cx: &mut Context<Self>) {
        self.output
            .recently_offered
            .retain(|_, offered| offered.elapsed() < OFFER_AGAIN_AFTER);

        let input = OfferInput {
            routing: self.output.routing_available,
            playing: self.playback.is_playing(),
            paused_by_disconnect: self.output.paused_by_disconnect.is_some(),
            soloist_device: self.output.soloist_device.as_deref(),
            offered_ago: self
                .output
                .recently_offered
                .get(&device.id)
                .map(Instant::elapsed),
        };

        if !should_offer(&device.id, &input) {
            debug!(
                name = %device.name,
                routing = input.routing,
                playing = input.playing,
                paused_by_disconnect = input.paused_by_disconnect,
                soloist_device = ?input.soloist_device,
                "not offering the new output device"
            );

            return;
        }

        debug!(name = %device.name, "offering the new output device");

        self.output
            .recently_offered
            .insert(device.id.clone(), Instant::now());
        self.show_output_popup(device, cx);
    }

    fn device_disconnected(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let moved_from = self
            .output
            .moved_from
            .as_ref()
            .map(|(id, at)| (id.as_str(), at.elapsed()));

        if !loses_soloist(
            id,
            self.output.soloist_device.as_deref(),
            moved_from,
            self.playback.is_playing(),
        ) {
            debug!(
                name,
                soloist_device = ?self.output.soloist_device,
                playing = self.playback.is_playing(),
                "an output device Soloist isn't playing on disconnected"
            );

            return;
        }

        debug!(name, "pausing: Soloist's output device disconnected");
        self.core.player.pause();
        self.output.paused_by_disconnect = Some(id.to_owned());
        self.output.disconnect_pause_seen = false;
        self.output.soloist_device = self.output.default_device.clone();
        self.show_toast(t!("store.toast.output_disconnected", name = name), cx);
    }

    /// Called with every playback update. Playing again after a pause that
    /// came from a disconnect means the user took over.
    pub(super) fn track_disconnect_pause(&mut self) {
        if self.output.paused_by_disconnect.is_none() {
            return;
        }

        if !self.playback.is_playing() {
            self.output.disconnect_pause_seen = true;
        } else if self.output.disconnect_pause_seen {
            self.output.paused_by_disconnect = None;
        }
    }

    /// The name of the device Soloist is playing on, if known.
    fn current_output_name(&self) -> Option<String> {
        let id = self.output.soloist_device.as_deref()?;

        self.output
            .devices
            .iter()
            .find(|device| same_device(Some(&device.id), Some(id)))
            .map(|device| device.name.clone())
    }

    fn show_output_popup(&mut self, device: OutputDevice, cx: &mut Context<Self>) {
        let previous = self.output.popup.take();
        let current = self
            .current_output_name()
            .unwrap_or_else(|| t!("output.popup.current").into_owned());
        let lifted = self.mini_player.is_some();
        let store = cx.entity();

        // The popup draws straight away and reads this store, which can't
        // happen while the store is being updated.
        cx.defer(move |cx| {
            if let Some(previous) = previous {
                let _ = previous.update(cx, |_, window, _| window.remove_window());
            }

            match output_popup::open(store.clone(), device, current, lifted, cx) {
                Ok(handle) => store.update(cx, |state, _| state.output.popup = Some(handle)),
                Err(error) => warn!(%error, "could not open the output device popup"),
            }
        });
    }

    fn close_output_popup(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.output.popup.take() else {
            return;
        };

        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
    }

    /// The popup's window is gone, however it was closed.
    pub fn output_popup_closed(&mut self, window_id: gpui_kit::WindowId) {
        if self
            .output
            .popup
            .is_some_and(|handle| handle.window_id() == window_id)
        {
            self.output.popup = None;
        }
    }

    /// Makes Soloist play on `device`, or on the Windows default with `None`.
    /// Soloist restarts, and playback resumes if it was playing.
    pub fn switch_output(&mut self, device: Option<String>, cx: &mut Context<Self>) {
        if self.output.switching {
            return;
        }

        let name = device.as_ref().and_then(|id| {
            self.output
                .devices
                .iter()
                .find(|known| same_device(Some(&known.id), Some(id)))
                .map(|known| known.name.clone())
                .or_else(|| self.core.config.get().output_device_name)
        });

        self.core.config.update(|config| {
            config.output_device.clone_from(&device);
            config.output_device_name = name;
        });
        self.close_output_popup(cx);

        let Some(manager) = self.core.soloist.clone() else {
            self.output.soloist_device = device.or_else(|| self.output.default_device.clone());
            cx.notify();

            return;
        };
        let Some(audio) = self
            .output
            .audio
            .clone()
            .filter(|_| self.output.routing_available)
        else {
            cx.notify();

            return;
        };

        let resume = self.playback.is_playing() || self.output.paused_by_disconnect.is_some();
        let plan = resume
            .then(|| {
                ResumePlan::capture(
                    &self.playback,
                    self.shown_position_ms(sonear_core::playback::now_ms()),
                )
            })
            .flatten();
        let pid = manager.supervisor().process_id();
        let player = self.core.player.clone();
        let events = self.core.subscribe();
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.output.switching = true;
        self.runtime.spawn(async move {
            let result =
                run_switch(audio, manager, player, events, pid, device.clone(), plan).await;

            let _ = done.send((device, result));
        });

        cx.spawn(async move |this, cx| {
            let Some((device, result)) = inbox.recv().await else {
                return;
            };

            let _ = this.update(cx, |store, cx| store.switch_finished(device, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn switch_finished(
        &mut self,
        device: Option<String>,
        result: Result<(), AudioError>,
        cx: &mut Context<Self>,
    ) {
        self.output.switching = false;
        self.output.paused_by_disconnect = None;

        match result {
            Ok(()) => {
                self.output.soloist_device = device.or_else(|| self.output.default_device.clone());
                self.output.checked_pid = None;
            }
            Err(error) => {
                warn!(%error, "could not change the output device");
                self.show_toast(t!("store.toast.output_failed"), cx);
            }
        }

        self.check_output_policy(cx);
        cx.notify();
    }

    /// Makes the Windows setting of a new Soloist process match the config,
    /// switching once if it doesn't. This covers a moved data folder (the
    /// program path changed) and edits in Windows Settings. Windows only
    /// answers once the process has opened audio, so this runs when Soloist's
    /// audio becomes active, and a mismatch goes through the normal switch so
    /// playback resumes.
    fn check_output_policy(&mut self, cx: &mut Context<Self>) {
        if self.output.switching
            || !self.output.routing_available
            || !matches!(
                self.soloist,
                Status::Ready { .. } | Status::NeedsPairing { .. }
            )
        {
            return;
        }

        let Some(audio) = self.output.audio.clone() else {
            return;
        };
        let Some(pid) = self
            .core
            .soloist
            .as_ref()
            .and_then(|manager| manager.supervisor().process_id())
        else {
            return;
        };

        if self.output.checked_pid == Some(pid) {
            return;
        }

        self.output.checked_pid = Some(pid);

        let wanted = self.core.config.get().output_device;
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.runtime.spawn(async move {
            let _ = done.send(check_policy(&audio, pid, wanted.as_deref()).await);
        });

        cx.spawn(async move |this, cx| {
            let Some(check) = inbox.recv().await else {
                return;
            };

            let _ = this.update(cx, |store, cx| store.policy_checked(check, cx));
        })
        .detach();
    }

    fn policy_checked(&mut self, check: Check, cx: &mut Context<Self>) {
        match check {
            Check::Matches => self.output.corrected = false,
            // Played before its audio was open; the next activation asks again.
            Check::Failed => self.output.checked_pid = None,
            Check::Mismatch if self.output.corrected => {
                warn!("the output device setting did not stick after a restart");
            }
            Check::Mismatch => {
                self.output.corrected = true;
                self.switch_output(self.core.config.get().output_device, cx);
            }
        }
    }

    /// Clears Windows' per-app setting for the running Soloist, before its
    /// data is deleted. Resolves once the audio thread has answered.
    pub(super) fn clear_output_route(&self) -> Option<impl Future<Output = ()> + use<>> {
        let audio = self.output.audio.clone()?;
        let pid = self.core.soloist.as_ref()?.supervisor().process_id()?;

        Some(async move {
            if let Err(error) = audio.route(pid, None).await {
                debug!(%error, "could not clear the output device setting");
            }
        })
    }
}

async fn check_policy(audio: &AudioHandle, pid: u32, wanted: Option<&str>) -> Check {
    match audio.persisted(pid).await {
        Ok(current) if same_device(current.as_deref(), wanted) => Check::Matches,
        Ok(_) => Check::Mismatch,
        Err(error) => {
            debug!(%error, "could not read the output device setting");

            Check::Failed
        }
    }
}

async fn run_switch(
    audio: AudioHandle,
    manager: Arc<SoloistManager>,
    player: Player,
    mut events: broadcast::Receiver<AppEvent>,
    pid: Option<u32>,
    device: Option<String>,
    plan: Option<ResumePlan>,
) -> Result<(), AudioError> {
    let Some(pid) = pid else {
        return Ok(());
    };

    audio.route(pid, device).await?;
    manager.restart().await;

    if let Some(plan) = plan {
        replay(&player, &mut events, &plan).await;
    }

    Ok(())
}

/// Plays the saved track again once the new Soloist is up. State comes from
/// Soloist's events only, so each step waits for the one before it.
async fn replay(player: &Player, events: &mut broadcast::Receiver<AppEvent>, plan: &ResumePlan) {
    let deadline = tokio::time::Instant::now() + RESUME_WAIT;

    if timeout_at(deadline, wait_for_state(events)).await != Ok(true) {
        debug!("gave up resuming: Soloist did not report its state");

        return;
    }

    match resume_call(&plan.track, plan.context.as_deref()) {
        ResumeCall::InContext { context, track } => {
            player.play_in_context(context, PlayOffset::Uri(track.to_owned()));
        }
        ResumeCall::Track(track) => player.play(Some(track)),
    }

    if plan.position_ms < SEEK_MIN_MS {
        return;
    }

    if timeout_at(deadline, wait_for_track(events, &plan.track)).await != Ok(true) {
        debug!("gave up resuming: the track did not start");

        return;
    }

    player.seek(plan.position_ms);
}

/// Waits for a Soloist that started after the call, and for its first state.
/// A state without a track is given a moment before it counts.
async fn wait_for_state(events: &mut broadcast::Receiver<AppEvent>) -> bool {
    let mut restarted = false;
    let mut ready = false;
    let mut have_track = false;
    let mut grace: Option<tokio::time::Instant> = None;

    loop {
        let event = match grace {
            Some(until) => match timeout_at(until, events.recv()).await {
                Ok(event) => event,
                Err(_) => return true,
            },
            None => events.recv().await,
        };

        match event {
            Ok(AppEvent::Soloist(Status::Ready { .. })) if restarted => {
                ready = true;
                grace = Some(tokio::time::Instant::now() + STATE_GRACE);
            }
            Ok(AppEvent::Soloist(Status::Ready { .. })) => {}
            Ok(AppEvent::Soloist(_)) => {
                restarted = true;
                ready = false;
                have_track = false;
                grace = None;
            }
            Ok(AppEvent::Playback(model)) if restarted => have_track = model.item.is_some(),
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return false,
        }

        if ready && have_track {
            return true;
        }
    }
}

/// Waits until the replayed track is really playing. The restarted Soloist
/// already reports that track, paused, and says "playing" before it loads the
/// new request, so a seek sent then is lost: the load has to be seen first.
async fn wait_for_track(events: &mut broadcast::Receiver<AppEvent>, track: &str) -> bool {
    let mut loading = false;

    loop {
        match events.recv().await {
            Ok(AppEvent::Playback(model)) => {
                let same_track = model.item.as_ref().is_some_and(|item| item.uri == track);

                if model.is_buffering() || !same_track {
                    loading = true;
                } else if loading && model.is_playing() {
                    return true;
                }
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADPHONES: &str = "{0.0.0.00000000}.{aaaaaaaa-0000-0000-0000-000000000001}";
    const SPEAKERS: &str = "{0.0.0.00000000}.{bbbbbbbb-0000-0000-0000-000000000002}";

    fn input() -> OfferInput<'static> {
        OfferInput {
            routing: true,
            playing: true,
            paused_by_disconnect: false,
            soloist_device: Some(SPEAKERS),
            offered_ago: None,
        }
    }

    #[test]
    fn offers_a_new_device_while_playing() {
        assert!(should_offer(HEADPHONES, &input()));
    }

    #[test]
    fn offers_after_a_disconnect_pause() {
        let input = OfferInput {
            playing: false,
            paused_by_disconnect: true,
            ..input()
        };

        assert!(should_offer(HEADPHONES, &input));
    }

    #[test]
    fn stays_quiet_when_paused_by_the_user() {
        let input = OfferInput {
            playing: false,
            ..input()
        };

        assert!(!should_offer(HEADPHONES, &input));
    }

    #[test]
    fn stays_quiet_without_routing() {
        let input = OfferInput {
            routing: false,
            ..input()
        };

        assert!(!should_offer(HEADPHONES, &input));
    }

    #[test]
    fn does_not_offer_the_device_already_in_use() {
        assert!(!should_offer(&SPEAKERS.to_uppercase(), &input()));
    }

    #[test]
    fn offers_when_the_current_device_is_unknown() {
        let input = OfferInput {
            soloist_device: None,
            ..input()
        };

        assert!(should_offer(HEADPHONES, &input));
    }

    #[test]
    fn does_not_offer_the_same_device_twice_in_a_row() {
        let recent = OfferInput {
            offered_ago: Some(Duration::from_secs(4)),
            ..input()
        };
        let old = OfferInput {
            offered_ago: Some(Duration::from_secs(11)),
            ..input()
        };

        assert!(!should_offer(HEADPHONES, &recent));
        assert!(should_offer(HEADPHONES, &old));
    }

    #[test]
    fn losing_the_current_device_while_playing_pauses() {
        assert!(loses_soloist(SPEAKERS, Some(SPEAKERS), None, true));
        assert!(!loses_soloist(SPEAKERS, Some(SPEAKERS), None, false));
        assert!(!loses_soloist(HEADPHONES, Some(SPEAKERS), None, true));
    }

    #[test]
    fn a_device_soloist_was_just_moved_off_still_counts() {
        let just_now = Some((HEADPHONES, Duration::from_secs(1)));
        let long_ago = Some((HEADPHONES, Duration::from_secs(30)));

        assert!(loses_soloist(HEADPHONES, Some(SPEAKERS), just_now, true));
        assert!(!loses_soloist(HEADPHONES, Some(SPEAKERS), long_ago, true));
    }

    #[test]
    fn a_context_keeps_its_place_and_a_lone_track_plays_alone() {
        let track = "spotify:track:abc";

        assert_eq!(
            resume_call(track, Some("spotify:album:xyz")),
            ResumeCall::InContext {
                context: "spotify:album:xyz",
                track
            }
        );
        assert_eq!(
            resume_call(track, Some("spotify:playlist:xyz")),
            ResumeCall::InContext {
                context: "spotify:playlist:xyz",
                track
            }
        );
        assert_eq!(resume_call(track, Some(track)), ResumeCall::Track(track));
        assert_eq!(resume_call(track, Some("")), ResumeCall::Track(track));
        assert_eq!(resume_call(track, None), ResumeCall::Track(track));
    }

    #[test]
    fn device_ids_compare_without_case_and_none_is_the_default() {
        assert!(same_device(None, None));
        assert!(same_device(Some("{A}"), Some("{a}")));
        assert!(!same_device(None, Some("{a}")));
        assert!(!same_device(Some("{a}"), Some("{b}")));
    }

    #[test]
    fn nothing_to_resume_without_a_track() {
        assert_eq!(ResumePlan::capture(&PlaybackModel::default(), 5_000), None);
    }
}
