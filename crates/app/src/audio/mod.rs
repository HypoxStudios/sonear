//! Everything Sonear needs from Windows audio, on one COM thread: naming
//! Soloist's mixer entry, listing output devices, noticing which one Soloist
//! plays on, and pinning Soloist to a device.
//!
//! Callbacks only forward messages to the thread, because registering or
//! unregistering from inside them can deadlock.

mod devices;
mod naming;
mod policy;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use thiserror::Error;
use tokio::sync::{mpsc as async_mpsc, oneshot};
use tracing::{debug, info, warn};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    AudioSessionDisconnectReason, AudioSessionState, AudioSessionStateActive,
    AudioSessionStateExpired, DEVICE_STATE, EDataFlow, ERole, IAudioSessionControl,
    IAudioSessionEvents, IAudioSessionEvents_Impl, IAudioSessionManager2,
    IAudioSessionNotification, IAudioSessionNotification_Impl, IMMDeviceEnumerator,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::{BOOL, GUID, HSTRING, PCWSTR};
use windows_core::implement;

use naming::{Naming, name_session};
use policy::Policy;

/// A connected output device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    /// The Windows `IMMDevice` ID.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioEvent {
    /// Whether Soloist can be pinned to a device on this Windows.
    Routing(bool),
    /// The device list or the default changed.
    Devices {
        devices: Vec<OutputDevice>,
        default: Option<String>,
    },
    /// A device became active after the first scan.
    Connected(OutputDevice),
    /// A device that was active is gone.
    Disconnected { id: String, name: String },
    /// Soloist's audio became active on this device.
    SoloistDevice(String),
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("the audio thread is not running")]
    Stopped,
    #[error("{0}")]
    Policy(String),
}

pub type AudioEvents = async_mpsc::UnboundedReceiver<AudioEvent>;

enum Message {
    Rescan,
    Route {
        pid: u32,
        device: Option<String>,
        reply: oneshot::Sender<Result<(), AudioError>>,
    },
    Persisted {
        pid: u32,
        reply: oneshot::Sender<Result<Option<String>, AudioError>>,
    },
    /// A Soloist session appeared on a device.
    Session {
        device_id: String,
        session: SendSession,
    },
    SessionActive {
        device_id: String,
    },
    SessionGone {
        serial: u64,
    },
}

/// A session control handed to the audio thread. The object lives in the
/// multithreaded apartment, so any of its threads may call it.
struct SendSession(IAudioSessionControl);

// SAFETY: see above; every thread that touches COM here is in the MTA.
unsafe impl Send for SendSession {}

/// Sends requests to the audio thread.
#[derive(Clone)]
pub struct AudioHandle {
    commands: Sender<Message>,
}

impl AudioHandle {
    /// Pins the process to `device`, or back to the Windows default with `None`.
    pub async fn route(&self, pid: u32, device: Option<String>) -> Result<(), AudioError> {
        let (reply, answer) = oneshot::channel();

        self.commands
            .send(Message::Route { pid, device, reply })
            .map_err(|_| AudioError::Stopped)?;

        answer.await.unwrap_or(Err(AudioError::Stopped))
    }

    /// The device Windows has stored for the process, `None` meaning the default.
    pub async fn persisted(&self, pid: u32) -> Result<Option<String>, AudioError> {
        let (reply, answer) = oneshot::channel();

        self.commands
            .send(Message::Persisted { pid, reply })
            .map_err(|_| AudioError::Stopped)?;

        answer.await.unwrap_or(Err(AudioError::Stopped))
    }
}

/// Starts the audio thread. `None` in demo builds or when it can't start.
pub fn start(soloist_exe: PathBuf) -> Option<(AudioHandle, AudioEvents)> {
    if cfg!(feature = "demo") {
        return None;
    }

    let (commands, inbox) = mpsc::channel();
    let (events, outbox) = async_mpsc::unbounded_channel();
    let thread_commands = commands.clone();
    let spawned = std::thread::Builder::new()
        .name("sonear-audio".into())
        .spawn(move || run(soloist_exe, thread_commands, inbox, events));

    if let Err(error) = spawned {
        warn!(%error, "could not start the audio thread");

        return None;
    }

    Some((AudioHandle { commands }, outbox))
}

#[implement(IMMNotificationClient)]
struct DeviceWatcher {
    tx: Sender<Message>,
}

impl DeviceWatcher {
    fn signal(&self) -> windows::core::Result<()> {
        let _ = self.tx.send(Message::Rescan);

        Ok(())
    }
}

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.signal()
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.signal()
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.signal()
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        _role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.signal()
    }

    fn OnPropertyValueChanged(
        &self,
        _id: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(IAudioSessionNotification)]
struct SessionWatcher {
    naming: Naming,
    device_id: String,
    tx: Sender<Message>,
}

impl IAudioSessionNotification_Impl for SessionWatcher_Impl {
    fn OnSessionCreated(
        &self,
        new_session: windows_core::Ref<IAudioSessionControl>,
    ) -> windows::core::Result<()> {
        if let Some(session) = new_session.as_ref()
            && name_session(session, &self.naming)
        {
            let _ = self.tx.send(Message::Session {
                device_id: self.device_id.clone(),
                session: SendSession(session.clone()),
            });
        }

        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
struct SessionEvents {
    device_id: String,
    serial: u64,
    tx: Sender<Message>,
}

impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(
        &self,
        _name: &PCWSTR,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnIconPathChanged(
        &self,
        _path: &PCWSTR,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnSimpleVolumeChanged(
        &self,
        _volume: f32,
        _mute: BOOL,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnChannelVolumeChanged(
        &self,
        _count: u32,
        _volumes: *const f32,
        _changed: u32,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnGroupingParamChanged(
        &self,
        _grouping: *const GUID,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnStateChanged(&self, state: AudioSessionState) -> windows::core::Result<()> {
        if state == AudioSessionStateActive {
            let _ = self.tx.send(Message::SessionActive {
                device_id: self.device_id.clone(),
            });
        } else if state == AudioSessionStateExpired {
            // The Soloist process behind it has exited, which happens on every restart.
            let _ = self.tx.send(Message::SessionGone {
                serial: self.serial,
            });
        }

        Ok(())
    }

    fn OnSessionDisconnected(
        &self,
        _reason: AudioSessionDisconnectReason,
    ) -> windows::core::Result<()> {
        let _ = self.tx.send(Message::SessionGone {
            serial: self.serial,
        });

        Ok(())
    }
}

struct SessionEntry {
    serial: u64,
    control: IAudioSessionControl,
    events: IAudioSessionEvents,
}

impl Drop for SessionEntry {
    fn drop(&mut self) {
        // SAFETY: both objects are alive and were registered together.
        let _ = unsafe {
            self.control
                .UnregisterAudioSessionNotification(&self.events)
        };
    }
}

struct Registration {
    manager: IAudioSessionManager2,
    notification: IAudioSessionNotification,
    sessions: Vec<SessionEntry>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.sessions.clear();

        // SAFETY: both objects are alive and were registered together.
        let _ = unsafe {
            self.manager
                .UnregisterSessionNotification(&self.notification)
        };
    }
}

struct Audio {
    enumerator: IMMDeviceEnumerator,
    naming: Naming,
    tx: Sender<Message>,
    events: async_mpsc::UnboundedSender<AudioEvent>,
    policy: Option<Policy>,
    devices: Vec<OutputDevice>,
    default: Option<String>,
    scanned: bool,
    registered: HashMap<String, Registration>,
    serial: u64,
}

fn run(
    soloist_exe: PathBuf,
    tx: Sender<Message>,
    inbox: Receiver<Message>,
    events: async_mpsc::UnboundedSender<AudioEvent>,
) {
    let icon = match std::env::current_exe() {
        Ok(exe) => Some(HSTRING::from(format!("{},0", exe.display()))),
        Err(error) => {
            warn!(%error, "could not resolve the app icon for the volume mixer");
            None
        }
    };
    let naming = Naming::new(soloist_exe, icon);

    // MTA, because the audio callbacks arrive on threads COM owns.
    // SAFETY: called once on this thread, which stays alive for the whole process.
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

    if initialised.is_err() {
        warn!(
            code = initialised.0,
            "could not initialise COM for the audio thread"
        );
        return;
    }

    // SAFETY: COM is initialised on this thread.
    let enumerator: IMMDeviceEnumerator =
        match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
            Ok(enumerator) => enumerator,
            Err(error) => {
                warn!(%error, "could not create the audio device enumerator");
                return;
            }
        };
    let watcher: IMMNotificationClient = DeviceWatcher { tx: tx.clone() }.into();

    // SAFETY: `watcher` is a valid callback object and lives until the process ends.
    if let Err(error) = unsafe { enumerator.RegisterEndpointNotificationCallback(&watcher) } {
        warn!(%error, "could not watch audio devices");
    }

    let policy = match Policy::open() {
        Ok(policy) => Some(policy),
        Err(error) => {
            warn!(%error, "choosing the output device is not available");
            None
        }
    };
    let _ = events.send(AudioEvent::Routing(policy.is_some()));

    let mut audio = Audio {
        enumerator,
        naming,
        tx,
        events,
        policy,
        devices: Vec::new(),
        default: None,
        scanned: false,
        registered: HashMap::new(),
        serial: 0,
    };

    audio.scan();

    while let Ok(first) = inbox.recv() {
        let mut dirty = audio.handle(first);

        while let Ok(next) = inbox.try_recv() {
            dirty |= audio.handle(next);
        }

        if dirty {
            audio.scan();
        }
    }
}

impl Audio {
    /// Handles one message. Returns whether the devices should be scanned again.
    fn handle(&mut self, message: Message) -> bool {
        match message {
            Message::Rescan => return true,
            Message::Route { pid, device, reply } => {
                let result = self.policy().and_then(|policy| {
                    policy
                        .set(pid, device.as_deref())
                        .map_err(|error| AudioError::Policy(error.to_string()))
                });

                let _ = reply.send(result);
            }
            Message::Persisted { pid, reply } => {
                let result = self.policy().and_then(|policy| {
                    policy
                        .get(pid)
                        .map_err(|error| AudioError::Policy(error.to_string()))
                });

                let _ = reply.send(result);
            }
            Message::Session { device_id, session } => self.add_session(&device_id, session.0),
            Message::SessionActive { device_id } => {
                let _ = self.events.send(AudioEvent::SoloistDevice(device_id));
            }
            Message::SessionGone { serial } => {
                for registration in self.registered.values_mut() {
                    registration.sessions.retain(|entry| entry.serial != serial);
                }
            }
        }

        false
    }

    fn policy(&self) -> Result<&Policy, AudioError> {
        self.policy
            .as_ref()
            .ok_or_else(|| AudioError::Policy("per-app output devices are not available".into()))
    }

    fn scan(&mut self) {
        // SAFETY: COM is initialised on this thread.
        let Some(current) = (unsafe { devices::list(&self.enumerator) }) else {
            return;
        };
        // SAFETY: as above.
        let default = unsafe { devices::default_id(&self.enumerator) };

        self.registered
            .retain(|id, _| current.iter().any(|device| &device.id == id));

        for device in &current {
            if !self.registered.contains_key(&device.id) {
                self.register(&device.id);
            }
        }

        let initial = !self.scanned;
        let changes = devices::diff(&self.devices, &current);
        let changed = current != self.devices || default != self.default;

        self.scanned = true;
        self.devices.clone_from(&current);
        self.default.clone_from(&default);

        if changed || initial {
            let _ = self.events.send(AudioEvent::Devices {
                devices: current,
                default,
            });
        }

        if initial {
            return;
        }

        for device in changes.added {
            info!(name = %device.name, "output device connected");
            let _ = self.events.send(AudioEvent::Connected(device));
        }

        for device in changes.removed {
            info!(name = %device.name, "output device disconnected");
            let _ = self.events.send(AudioEvent::Disconnected {
                id: device.id,
                name: device.name,
            });
        }
    }

    fn register(&mut self, id: &str) {
        let wide = HSTRING::from(id);

        // SAFETY: plain COM calls on an initialised thread.
        let manager = unsafe {
            self.enumerator
                .GetDevice(&wide)
                .and_then(|device| device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None))
        };
        let manager = match manager {
            Ok(manager) => manager,
            Err(error) => {
                debug!(%error, "could not open a device's audio sessions");
                return;
            }
        };
        let notification: IAudioSessionNotification = SessionWatcher {
            naming: self.naming.clone(),
            device_id: id.to_owned(),
            tx: self.tx.clone(),
        }
        .into();

        // Session notifications only start once the enumerator has been fetched.
        // SAFETY: plain call on a live object.
        let sessions = match unsafe { manager.GetSessionEnumerator() } {
            Ok(sessions) => sessions,
            Err(error) => {
                debug!(%error, "could not list a device's audio sessions");
                return;
            }
        };
        // SAFETY: plain call on a live object.
        let count = unsafe { sessions.GetCount() }.unwrap_or(0);
        let mut found = Vec::new();

        for index in 0..count {
            // SAFETY: plain call on a live object.
            if let Ok(session) = unsafe { sessions.GetSession(index) }
                && name_session(&session, &self.naming)
                && let Some(entry) = self.watch(id, session)
            {
                found.push(entry);
            }
        }

        // SAFETY: `notification` is kept alive in the registration.
        if let Err(error) = unsafe { manager.RegisterSessionNotification(&notification) } {
            debug!(%error, "could not watch a device's audio sessions");
            return;
        }

        self.registered.insert(
            id.to_owned(),
            Registration {
                manager,
                notification,
                sessions: found,
            },
        );
    }

    fn add_session(&mut self, device_id: &str, session: IAudioSessionControl) {
        if !self.registered.contains_key(device_id) {
            return;
        }

        if let Some(entry) = self.watch(device_id, session)
            && let Some(registration) = self.registered.get_mut(device_id)
        {
            registration.sessions.push(entry);
        }
    }

    /// Listens to a Soloist session's state. A session that is already
    /// playing counts at once, because its first change was missed.
    fn watch(&mut self, device_id: &str, control: IAudioSessionControl) -> Option<SessionEntry> {
        self.serial += 1;

        let serial = self.serial;
        let events: IAudioSessionEvents = SessionEvents {
            device_id: device_id.to_owned(),
            serial,
            tx: self.tx.clone(),
        }
        .into();

        // SAFETY: plain calls on live objects; `events` is kept in the entry.
        unsafe {
            control
                .RegisterAudioSessionNotification(&events)
                .inspect_err(|error| debug!(%error, "could not watch a Soloist audio session"))
                .ok()?;

            if control
                .GetState()
                .is_ok_and(|state| state == AudioSessionStateActive)
            {
                let _ = self
                    .events
                    .send(AudioEvent::SoloistDevice(device_id.to_owned()));
            }
        }

        Some(SessionEntry {
            serial,
            control,
            events,
        })
    }
}
