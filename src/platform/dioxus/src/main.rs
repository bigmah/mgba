//! mGBA × Dioxus: a library-first desktop frontend for mGBA.

mod art;
mod emu;
mod ffi;
mod library;
mod ui_library;
mod ui_player;

use dioxus::desktop::{
    tao::event::{Event, WindowEvent},
    use_asset_handler, use_wry_event_handler, Config, LogicalSize, WindowBuilder,
};
use dioxus::prelude::*;
use library::{now_secs, Game, Store};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const CSS: &str = include_str!("../assets/style.css");
const JS: &str = include_str!("../assets/player.js");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--selftest") {
        return selftest(
            args.get(2).expect("usage: --selftest <rom> [state]"),
            args.get(3).map(PathBuf::from),
        );
    }
    if args.get(1).map(String::as_str) == Some("--list") {
        for g in library::scan(std::path::Path::new(args.get(2).expect("usage: --list <dir>"))) {
            println!(
                "{}\t{}\t{:?}\t{:?}\t{:?}\t{:?}\t{}",
                g.letter, g.title, g.regions, g.languages, g.revision, g.tags, g.header.game_code
            );
        }
        return;
    }

    let cfg = Config::new()
        .with_window(
            WindowBuilder::new()
                .with_title("mGBA")
                .with_inner_size(LogicalSize::new(1440.0, 920.0))
                .with_min_inner_size(LogicalSize::new(960.0, 620.0)),
        )
        .with_custom_head(format!("<style>{CSS}</style><script>{JS}</script>"))
        .with_background_color((11, 10, 20, 255))
        .with_disable_context_menu(!cfg!(debug_assertions));
    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(App);
}

/// Headless smoke test of the core: `mgba-dioxus --selftest <rom> [state]`.
fn selftest(rom: &str, state: Option<PathBuf>) {
    let rom = PathBuf::from(rom);
    let mut core = ffi::Core::open(&rom, None).expect("mGBA failed to load ROM");
    if let Some(state) = state {
        println!("load external state: {}", core.load_state(&state));
    }
    let mut audio = vec![0i16; 8192];
    let mut samples = 0;
    for i in 0..600 {
        core.set_keys(if (300..310).contains(&i) { ffi::keys::START } else { 0 });
        core.run_frame();
        samples += core.read_audio(&mut audio);
    }
    let mut buf = Vec::new();
    let (w, h) = core.frame_rgba(&mut buf);
    let lit = buf.chunks(4).filter(|p| p[0] | p[1] | p[2] != 0).count();
    let tmp = std::env::temp_dir();
    println!("video {w}x{h} @ {:.4} fps, {lit} lit pixels", core.fps());
    println!("audio {} Hz, {samples} frames over 600 video frames", core.sample_rate());
    println!("screenshot: {}", core.screenshot(&tmp.join("mgba-selftest.png")));
    println!("save state: {}", core.save_state(&tmp.join("mgba-selftest.ss1")));
    println!("load state: {}", core.load_state(&tmp.join("mgba-selftest.ss1")));
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct AppState {
    pub games: Signal<Arc<Vec<Game>>>,
    pub roms_dir: Signal<Option<PathBuf>>,
    pub store: Signal<Store>,
    pub session: Signal<Option<emu::Session>>,
    pub playing: Signal<Option<usize>>,
    pub selected: Signal<Option<usize>>,
    pub toasts: Signal<Vec<(u64, String)>>,
    /// Bumped when saves/states on disk may have changed.
    pub tick: Signal<u64>,
}

impl AppState {
    pub fn game(&self, id: usize) -> Option<Game> {
        self.games.peek().get(id).cloned()
    }

    pub fn toast(mut self, msg: impl Into<String>) {
        let id = now_nanos();
        self.toasts.write().push((id, msg.into()));
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(2600)).await;
            self.toasts.write().retain(|t| t.0 != id);
        });
    }

    /// Start playing `id`. With `resume`, continue from the automatic exit state.
    pub fn launch(mut self, id: usize, resume: bool) -> bool {
        let Some(game) = self.game(id) else { return false };
        self.quit_game();
        let resume_file = library::resume_path(&game);
        let resume_from = (resume && resume_file.exists()).then_some(resume_file);
        let volume = self.store.peek().settings.volume;
        match emu::Session::start(game.clone(), resume_from, volume) {
            Ok(session) => {
                self.session.set(Some(session));
                self.playing.set(Some(id));
                self.selected.set(None);
                let mut store = self.store.write();
                let stats = store.stats.entry(game.stem.clone()).or_default();
                stats.plays += 1;
                stats.last_played = now_secs();
                store.save();
                true
            }
            Err(e) => {
                self.toast(e);
                false
            }
        }
    }

    /// Stop the running game (writes the resume state and flushes the battery save).
    pub fn quit_game(mut self) {
        let Some(session) = self.session.write().take() else { return };
        let secs = session.started.elapsed().as_secs();
        let stem = session.game.stem.clone();
        drop(session);
        {
            let mut store = self.store.write();
            let stats = store.stats.entry(stem).or_default();
            stats.seconds += secs;
            stats.last_played = now_secs();
            store.save();
        }
        self.playing.set(None);
        *self.tick.write() += 1;
    }

    pub fn toggle_favorite(mut self, id: usize) {
        let Some(game) = self.game(id) else { return };
        let mut store = self.store.write();
        if !store.favorites.remove(&game.stem) {
            store.favorites.insert(game.stem);
        }
        store.save();
    }

    pub fn set_roms_dir(mut self, dir: PathBuf) {
        let games = library::scan(&dir);
        let n = games.len();
        self.games.set(Arc::new(games));
        self.roms_dir.set(Some(dir.clone()));
        self.selected.set(None);
        {
            let mut store = self.store.write();
            store.settings.roms_dir = Some(dir);
            store.save();
        }
        self.toast(format!("Found {n} games"));
    }

    pub fn update_settings(mut self, f: impl FnOnce(&mut library::Settings)) {
        let mut store = self.store.write();
        f(&mut store.settings);
        store.save();
    }
}

pub fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

pub fn reveal_in_finder(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
}

#[component]
fn App() -> Element {
    let state = use_context_provider(|| {
        let store = Store::load();
        let dir = library::resolve_roms_dir(&store);
        let games = dir.as_deref().map(library::scan).unwrap_or_default();
        AppState {
            games: Signal::new(Arc::new(games)),
            roms_dir: Signal::new(dir),
            store: Signal::new(store),
            session: Signal::new(None),
            playing: Signal::new(None),
            selected: Signal::new(None),
            toasts: Signal::new(Vec::new()),
            tick: Signal::new(0),
        }
    });

    // `/art/<boxart|title|snap>/<id>`, `/art/state/<id>/<slot>`, `/art/resume/<id>`
    let art = use_hook(art::ArtService::new);
    use_asset_handler("art", move |req, responder| {
        let path = req.uri().path().to_string();
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let game = |s: &str| s.parse::<usize>().ok().and_then(|i| state.game(i));
        match parts.as_slice() {
            ["art", "state", id, slot] => {
                let bytes = game(id).and_then(|g| {
                    std::fs::read(g.state_path(slot.parse().unwrap_or(1))).ok()
                });
                art::respond_image(responder, bytes, "image/png", false);
            }
            ["art", "resume", id] => {
                let bytes = game(id).and_then(|g| std::fs::read(library::resume_path(&g)).ok());
                art::respond_image(responder, bytes, "image/png", false);
            }
            ["art", kind, id] => match (art::ArtKind::parse(kind), game(id)) {
                (Some(kind), Some(g)) => {
                    art.request(kind, &g.stem, g.platform, g.is_translation(), responder)
                }
                _ => art::respond_image(responder, None, "image/png", false),
            },
            _ => art::respond_image(responder, None, "image/png", false),
        }
    });
    use_asset_handler("emu", move |req, responder| {
        emu::serve_frame(req.uri().query(), responder)
    });

    // Make sure progress is written if the window is closed mid-game.
    use_wry_event_handler(move |event, _| {
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested | WindowEvent::Destroyed,
            ..
        } = event
        {
            state.quit_game();
        }
    });

    // `MGBA_SELFCHECK=<game id>`: drive the real UI end-to-end (render the
    // library, launch a game, inspect the canvas, save/load a state, quit)
    // and log what the webview actually shows. Useful for CI and debugging.
    use_future(move || async move {
        let Some(id) = std::env::var("MGBA_SELFCHECK").ok().and_then(|v| v.parse::<usize>().ok()) else {
            return;
        };
        let probe = |js: &'static str| async move {
            document::eval(js).join::<serde_json::Value>().await.unwrap_or_default()
        };
        tokio::time::sleep(Duration::from_secs(4)).await;
        let lib = probe(r#"return {
            cards: document.querySelectorAll('.card').length,
            art_loaded: document.querySelectorAll('img.art.loaded').length,
            sidebar: [...document.querySelectorAll('.nav-item .label')].slice(0, 6).map(e => e.textContent),
            hero: document.querySelector('.hero h1')?.textContent,
            heading: document.querySelector('.section-head h2')?.textContent,
        };"#).await;
        eprintln!("SELFCHECK library: {lib}");
        state.selected.clone().set(Some(id));
        tokio::time::sleep(Duration::from_millis(800)).await;
        let drawer = probe(r#"return {
            title: document.querySelector('.drawer h2')?.textContent,
            facts: document.querySelectorAll('.facts dt').length,
            actions: [...document.querySelectorAll('.drawer-actions button')].map(b => b.textContent.trim()),
        };"#).await;
        eprintln!("SELFCHECK drawer: {drawer}");
        let ok = state.launch(id, false);
        eprintln!("SELFCHECK launch {id}: {ok}");
        tokio::time::sleep(Duration::from_secs(10)).await;
        let fetch = probe(r#"try {
                const r = await fetch('/emu/frame?s=0', { cache: 'no-store' });
                const b = await r.arrayBuffer();
                return { status: r.status, bytes: b.byteLength };
            } catch (e) { return { error: String(e) }; }"#).await;
        eprintln!("SELFCHECK fetch: {fetch}");
        let dbg = probe("return typeof window.__gba === 'object' ? window.__gba.debug() : 'missing'").await;
        eprintln!("SELFCHECK js: {dbg}");
        let canvas = probe(r#"const c = document.getElementById('gba-screen');
            if (!c) return { canvas: false };
            const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
            let lit = 0, colors = new Set();
            for (let i = 0; i < d.length; i += 4) { if (d[i] | d[i+1] | d[i+2]) lit++; if (i % 64 == 0) colors.add(d[i] << 16 | d[i+1] << 8 | d[i+2]); }
            return { w: c.width, h: c.height, css: c.style.width + 'x' + c.style.height, lit, colors: colors.size,
                     focused: document.activeElement?.id, fps: document.querySelector('.fps')?.textContent };"#).await;
        eprintln!("SELFCHECK canvas: {canvas}");
        let key = |code: &'static str| {
            document::eval(&format!(
                "document.getElementById('player-root').dispatchEvent(new KeyboardEvent('keydown', {{ key: '{code}', code: '{code}', bubbles: true }}))"
            ));
        };
        key("Escape");
        tokio::time::sleep(Duration::from_millis(600)).await;
        let paused = probe(r#"const btn = [...document.querySelectorAll('.menu-btn')].find(b => b.textContent.includes('Save state'));
            btn?.click();
            return { pause: !!document.querySelector('.pause'), buttons: document.querySelectorAll('.menu-btn').length };"#).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let slots = probe("return document.querySelectorAll('.pause .slot').length").await;
        eprintln!("SELFCHECK pause: {paused} slots={slots}");
        key("Escape");
        key("Escape");
        tokio::time::sleep(Duration::from_millis(600)).await;
        let resumed = probe("return !document.querySelector('.pause')").await;
        eprintln!("SELFCHECK resumed: {resumed}");
        if let Some(s) = state.session.peek().as_ref() {
            s.send(emu::Cmd::SaveState(9));
            s.send(emu::Cmd::LoadState(9));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let toasts: Vec<String> = state.toasts.peek().iter().map(|t| t.1.clone()).collect();
        eprintln!("SELFCHECK toasts: {toasts:?}");
        state.quit_game();
        let game = state.game(id).unwrap();
        eprintln!(
            "SELFCHECK quit: resume={} slot9={}",
            library::resume_path(&game).exists(),
            game.state_path(9).exists()
        );
        let _ = std::fs::remove_file(game.state_path(9));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let hero = probe("return document.querySelector('.hero-kicker')?.textContent").await;
        eprintln!("SELFCHECK hero after quit: {hero}");
        std::process::exit(0);
    });

    rsx! {
        ui_library::Library {}
        if state.playing.read().is_some() {
            ui_player::Player {}
        }
        div { class: "toasts",
            for (id, msg) in state.toasts.read().iter().cloned() {
                div { key: "{id}", class: "toast", "{msg}" }
            }
        }
    }
}
