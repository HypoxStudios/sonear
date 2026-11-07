#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod cover_cache;
mod data;
mod format;
mod http;
mod instance;
mod store;
mod theme;
mod ui;

use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use sonear_core::{AppPaths, Core};
use tracing::{error, info};

const DEMO: bool = cfg!(feature = "demo");
/// Decoded cover images kept in memory at once.
const IMAGE_CACHE_CAPACITY: usize = 160;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "sonear=info,sonear_core=info,sonear_soloist=info,sonear_spotify=info".into()
            }),
        )
        .init();

    let Some(_instance) = instance::acquire(DEMO) else {
        info!("Sonear is already running");
        return;
    };
    let Some(paths) = AppPaths::discover() else {
        error!("could not locate the user profile directories");
        return;
    };
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
    let core = Arc::new(start_core(&handle, &paths));
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
            cx.set_http_client(http::ReqwestClient::new(handle.clone(), covers));
            gpui_kit::init(cx);
            theme::apply(cx);
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
                ..TitleBar::window_options()
            };

            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| ui::shell::Shell::new(store, window, cx))
            });

            if let Err(error) = opened {
                error!(%error, "could not open the window");
                cx.quit();
            }

            cx.activate(true);
        });

    drop(runtime);
}

#[cfg(feature = "demo")]
fn start_core(runtime: &tokio::runtime::Handle, paths: &AppPaths) -> Core {
    let config = sonear_core::ConfigHandle::load(paths.config_dir().join("config.demo.toml"));

    Core::start_demo(runtime, config)
}

#[cfg(not(feature = "demo"))]
fn start_core(runtime: &tokio::runtime::Handle, paths: &AppPaths) -> Core {
    Core::start(runtime, paths)
}
