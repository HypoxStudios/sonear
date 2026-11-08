#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app_identity;
mod assets;
mod audio_session;
mod cover_accent;
mod cover_cache;
mod data;
mod data_mover;
mod format;
mod http;
mod i18n;
mod installer;
mod instance;
mod keymap;
mod launch;
mod legal;
mod media_controls;
mod platform_window;
mod protocol;
mod site;
mod store;
mod theme;
mod tray;
mod ui;

rust_i18n::i18n!("locales", fallback = "en");

use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AppContext as _, AsyncApp, Bounds, Entity, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use instance::Handoff;
use launch::Launch;
use media_controls::{MediaCommand, MediaControls};
use sonear_core::{AppPaths, Core, SpotifyLink};
use store::AppStore;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use tray::TrayCommand;

const DEMO: bool = cfg!(feature = "demo");
const WINDOW_TITLE: &str = "Sonear";
/// Decoded cover images kept in memory at once.
const IMAGE_CACHE_CAPACITY: usize = 160;

/// The GUI subsystem starts without a console, so a launch from a terminal
/// would lose its logs. Borrow the parent's console unless stdout is already
/// redirected.
#[cfg(windows)]
fn attach_parent_console() {
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };

    // SAFETY: both calls take plain values and have no preconditions.
    unsafe {
        let usable = GetStdHandle(STD_OUTPUT_HANDLE)
            .is_ok_and(|handle| !handle.is_invalid() && !handle.0.is_null());

        if !usable {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn main() {
    attach_parent_console();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "sonear=info,sonear_core=info,sonear_soloist=info,sonear_spotify=info".into()
            }),
        )
        .init();

    i18n::apply_saved();

    let exe = std::env::current_exe().unwrap_or_default();
    let parsed = launch::parse(std::env::args_os().skip(1), &exe);

    if let Some(pid) = parsed.after_pid
        && !matches!(parsed.launch, Launch::MoveData(_) | Launch::ApplyUpdate(_))
    {
        launch::wait_for_exit(pid);
    }

    match parsed.launch {
        Launch::Setup => installer::run_setup(),
        Launch::Uninstall => installer::run_uninstall(),
        Launch::UninstallFrom(dir) => installer::run_uninstall_from(dir),
        Launch::MoveData(target) => data_mover::run(target, parsed.after_pid),
        Launch::ApplyUpdate(dir) => installer::apply::run(dir, parsed.after_pid),
        #[cfg(feature = "demo")]
        Launch::DataMissing => {
            data_mover::run_missing(std::path::PathBuf::from(r"D:\Sonear"), None)
        }
        Launch::App { link, ignored } => run_app(link, ignored),
    }
}

fn run_app(link: Option<SpotifyLink>, ignored: Option<String>) {
    if let Some(shown) = ignored {
        info!(argument = %shown, "ignored a launch argument that isn't a Spotify link");
    }

    let Some(guard) = instance::acquire(DEMO) else {
        info!("Sonear is already running");
        instance::forward(DEMO, &link.map_or(Handoff::Show, Handoff::Open));

        return;
    };
    let Some(paths) = AppPaths::discover() else {
        error!("could not locate the user profile directories");
        return;
    };

    if !DEMO && data_folder_missing(&paths) {
        warn!(folder = %paths.local_root().display(), "the configured data folder is missing");
        data_mover::run_missing(paths.local_root(), Some(guard));

        return;
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("sonear-io")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            error!(%error, "could not start the async runtime");
            return;
        }
    };
    let handle = runtime.handle().clone();
    let handoffs = instance::serve(&handle, DEMO);
    let core = Arc::new(start_core(&handle, &paths));

    i18n::apply(i18n::initial(&core.config.get()));

    let covers = (!DEMO).then(|| {
        cover_cache::DiskCache::open(
            paths.cover_cache_dir(),
            cover_cache::MAX_BYTES,
            cover_cache::MAX_AGE,
        )
    });

    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx| {
            cx.set_app_identity(app_identity::APP_ID, app_identity::APP_NAME);
            app_identity::install();

            if let Some(soloist) = &core.soloist {
                audio_session::start(soloist.supervisor().dirs().exe());
            }

            cx.set_http_client(http::ReqwestClient::new(handle.clone(), covers));
            gpui_kit::init(cx);
            theme::apply(cx);
            keymap::bind_fixed(cx);
            keymap::apply(cx, &[], &keymap::effective(&core.config.get().shortcuts));
            ui::widgets::set_image_cache(
                cx.new(|_| cover_cache::LruImageCache::new(IMAGE_CACHE_CAPACITY)),
            );

            let store = store::new_entity(core.clone(), handle.clone(), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(800.)),
                    cx,
                ))),
                window_min_size: Some(size(px(900.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::new_static(WINDOW_TITLE)),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            };

            let shell_store = store.clone();
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| ui::shell::Shell::new(shell_store, window, cx))
            });

            let Ok((window, _)) = opened else {
                if let Err(error) = opened {
                    error!(%error, "could not open the window");
                }

                cx.quit();

                return;
            };

            store.update(cx, |store, _| store.set_main_window(window));
            platform_window::focus(window, cx);

            let close_store = store.clone();
            let _ = window.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, move |window, cx| {
                    close_store.update(cx, |store, cx| store.request_close(window, cx))
                });
            });

            start_tray(&store, cx);
            start_media_controls(&store, window, cx);

            if let Some(link) = link.clone() {
                store.update(cx, |store, cx| store.open_link(link, cx));
            }

            if let Some(mut inbox) = handoffs {
                cx.spawn(async move |cx| {
                    while let Some(handoff) = inbox.recv().await {
                        match handoff {
                            Handoff::Quit => {
                                cx.update(|cx| cx.quit());

                                break;
                            }
                            Handoff::Open(link) => {
                                store.update(cx, |store, cx| store.open_link(link, cx));
                            }
                            Handoff::Show => {}
                        }

                        store.update(cx, |store, cx| store.show_main_window(cx));
                    }
                })
                .detach();
            }

            cx.activate(true);
        });

    drop(runtime);
}

/// Whether the data folder was moved off the default and is no longer there.
fn data_folder_missing(paths: &AppPaths) -> bool {
    let moved = AppPaths::default_local_root()
        .is_some_and(|default| !sonear_core::data_dir::same_path(&default, &paths.local_root()));

    moved && !paths.local_root().exists()
}

/// Adds the tray icon and applies its menu choices. A tray that fails to start
/// leaves the app without one, and closing the window quits instead of hiding.
fn start_tray(store: &Entity<AppStore>, cx: &mut gpui_kit::App) {
    let (commands, mut inbox) = mpsc::unbounded_channel();

    if let Err(error) = tray::install(commands) {
        warn!(%error, "could not add the tray icon");

        return;
    }

    cx.on_app_quit(|_| async { tray::remove() }).detach();

    let store = store.clone();

    cx.spawn(async move |cx| {
        while let Some(command) = inbox.recv().await {
            run_tray_command(command, &store, cx);
        }
    })
    .detach();
}

fn run_tray_command(command: TrayCommand, store: &Entity<AppStore>, cx: &mut AsyncApp) {
    store.update(cx, |store, cx| match command {
        TrayCommand::Show => store.show_main_window(cx),
        TrayCommand::ToggleMiniPlayer => store.toggle_mini_player(cx),
        TrayCommand::Quit => cx.quit(),
        TrayCommand::TogglePlayPause => store.core.player.toggle_play_pause(),
        TrayCommand::Next => store.core.queue.next(),
        TrayCommand::Previous => store.core.player.previous(),
    });
}

/// Hooks the system media controls up to the main window and the store.
fn start_media_controls(
    store: &Entity<AppStore>,
    window: gpui_kit::AnyWindowHandle,
    cx: &mut gpui_kit::App,
) {
    let Some(hwnd) = window
        .update(cx, |_, window, _| platform_window::hwnd(window))
        .ok()
        .flatten()
    else {
        warn!("could not find the main window for the media controls");

        return;
    };
    let (commands, mut inbox) = mpsc::unbounded_channel();
    let mut media = match MediaControls::new(hwnd, commands) {
        Ok(media) => media,
        Err(error) => {
            warn!(%error, "could not start the media controls");

            return;
        }
    };

    media.update(&store.read(cx).playback);
    cx.observe(store, move |store, cx| {
        media.update(&store.read(cx).playback)
    })
    .detach();

    let store = store.clone();

    cx.spawn(async move |cx| {
        while let Some(command) = inbox.recv().await {
            run_media_command(command, &store, cx);
        }
    })
    .detach();
}

fn run_media_command(command: MediaCommand, store: &Entity<AppStore>, cx: &mut AsyncApp) {
    store.update(cx, |store, cx| {
        let player = &store.core.player;

        match command {
            MediaCommand::Play => player.play(None),
            MediaCommand::Pause | MediaCommand::Stop => player.pause(),
            MediaCommand::Next => store.core.queue.next(),
            MediaCommand::Previous => player.previous(),
            MediaCommand::Seek(position) => {
                if store.playback.duration_ms().is_some() {
                    store.seek(position, cx);
                }
            }
        }
    });
}

#[cfg(feature = "demo")]
fn start_core(runtime: &tokio::runtime::Handle, paths: &AppPaths) -> Core {
    let config = sonear_core::ConfigHandle::load(paths.config_dir().join(i18n::CONFIG_FILE));

    Core::start_demo(runtime, config)
}

#[cfg(not(feature = "demo"))]
fn start_core(runtime: &tokio::runtime::Handle, paths: &AppPaths) -> Core {
    Core::start(runtime, paths)
}
