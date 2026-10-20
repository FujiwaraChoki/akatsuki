//! # akatsuki-desktop
//!
//! The Akatsuki desktop app (GPUI 0.2.2), ARCHITECTURE §6.3 and §16.3: a keyboard-first mail
//! client over a full SQLite metadata mirror, with native rendering of sanitized mail, local FTS5
//! search, optimistic actions with undo, compose with undo send, and BYO AI.
//!
//! - `--demo` (or `AKATSUKI_DEMO=1`) opens the demo mailbox: invented mail, nothing leaves the
//!   machine, nothing is written to disk or the keychain.
//! - `AKATSUKI_EXIT_AFTER_MS=<n>` closes the app after `n` milliseconds (smoke tests and CI).
//! - `AKATSUKI_CONVEX_URL`, `AKATSUKI_CONVEX_SITE_URL`, `AKATSUKI_SYNC_URL`, `AKATSUKI_API_URL`
//!   point it at a deployment (or `desktop.json` in the config directory); `AKATSUKI_DATA_DIR`
//!   moves the data directory.
//! - `AKATSUKI_SCRIPT="go.starred;nav.open;search=teapot"` runs command ids one after another at
//!   start-up, as if typed (`search=<q>` types a search, `attach=<path>` drops a file on the open
//!   composer), for smoke tests without a keyboard.

mod assets;
mod auth;
mod backend;
mod commands;
mod config;
mod demo;
mod keys;
mod remote;
mod render;
mod secrets;
mod theme;
mod ui;
mod vm;
mod worker;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use akatsuki_core::store::Store;
use akatsuki_search::SearchIndex;

use crate::backend::Backend;
use crate::config::{Endpoints, LocalSettings, Paths};
use crate::secrets::{Keychain, MemoryStore, SecretStore};

fn open_store(paths: &Paths, demo: bool) -> (Store, SearchIndex) {
    if demo {
        return (
            Store::open_in_memory().expect("in-memory mirror"),
            SearchIndex::open_in_memory().expect("in-memory index"),
        );
    }
    let _ = std::fs::create_dir_all(&paths.data_dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&paths.data_dir, std::fs::Permissions::from_mode(0o700));
    }
    let store = Store::open(&paths.mirror()).unwrap_or_else(|e| {
        eprintln!("akatsuki-desktop: the mirror could not be opened ({e}); using memory for this session");
        Store::open_in_memory().expect("in-memory mirror")
    });
    let search = SearchIndex::open(&paths.search()).unwrap_or_else(|e| {
        eprintln!("akatsuki-desktop: the search index could not be opened ({e}); using memory for this session");
        SearchIndex::open_in_memory().expect("in-memory index")
    });
    (store, search)
}

fn main() {
    let exit_after = std::env::var("AKATSUKI_EXIT_AFTER_MS").ok().and_then(|v| v.parse::<u64>().ok());
    let demo = std::env::args().any(|a| a == "--demo") || std::env::var("AKATSUKI_DEMO").is_ok_and(|v| v == "1");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("akatsuki-io")
        .build()
        .expect("tokio runtime");
    let paths = Paths::detect();
    let settings = LocalSettings::load(&paths.config_file);
    let endpoints = Endpoints::resolve(&settings.endpoints, |name| std::env::var(name).ok());
    let device_id = if demo { "demo-device".to_owned() } else { config::device_id(&paths.data_dir) };
    render::original::clear_originals(&paths.originals());
    let (store, search) = open_store(&paths, demo);
    let secrets: Arc<dyn SecretStore> = if demo { Arc::new(MemoryStore::default()) } else { Arc::new(Keychain) };
    let (backend, events) = Backend::new(
        rt.handle().clone(),
        store,
        search,
        secrets,
        endpoints,
        paths.clone(),
        device_id,
        settings.cache_bodies && !demo,
    );

    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(assets::FONTS.iter().map(|(_, bytes)| Cow::Borrowed(*bytes)).collect())
            .expect("bundled fonts are valid");
        ui::input::bind_keys(cx);

        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        let b = backend.clone();
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("Akatsuki".into()), ..Default::default() }),
                app_id: Some("vc.akatsuki.desktop".into()),
                window_min_size: Some(size(px(720.), px(480.))),
                ..Default::default()
            },
            move |window, cx| {
                // The Wayland backend ignores TitlebarOptions::title; set it explicitly.
                window.set_window_title("Akatsuki");
                cx.new(|cx| ui::workspace::Workspace::new(b, events, settings, paths, window, cx))
            },
        );
        let window = match window {
            Ok(w) => w,
            Err(err) => {
                eprintln!("akatsuki-desktop: could not open a window: {err:#}");
                cx.quit();
                return;
            }
        };
        if let Ok(script) = std::env::var("AKATSUKI_SCRIPT") {
            let steps: Vec<String> = script.split(';').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(1500)).await;
                for step in steps {
                    let _ = window.update(cx, |ws, window, cx| {
                        if let Some(q) = step.strip_prefix("search=") {
                            ws.search_for(q, window, cx);
                        } else if let Some(p) = step.strip_prefix("attach=") {
                            ws.attach_paths(vec![p.into()], window, cx);
                        } else {
                            ws.run(&step, window, cx);
                        }
                    });
                    cx.background_executor().timer(Duration::from_millis(600)).await;
                }
            })
            .detach();
        }
        if demo {
            backend.start_demo();
        } else {
            backend.start();
            backend.reindex_all();
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if let Some(ms) = exit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
        cx.activate(true);
    });
    rt.shutdown_timeout(Duration::from_millis(500));
}
