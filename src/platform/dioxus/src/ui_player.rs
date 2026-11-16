//! In-game view: the canvas fed by `/emu/frame`, a HUD, keyboard input, and a
//! glassy pause menu with save-state slots (with screenshots), display and
//! sound options.

use crate::emu::{Cmd, Shared};
use crate::ffi::keys;
use crate::library::STATE_SLOTS;
use crate::ui_library::{ago, ControlsTable};
use crate::AppState;
use dioxus::prelude::*;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Home,
    Save,
    Load,
    Display,
    Controls,
}

fn key_bit(code: &Code) -> Option<u32> {
    Some(match code {
        Code::ArrowUp => keys::UP,
        Code::ArrowDown => keys::DOWN,
        Code::ArrowLeft => keys::LEFT,
        Code::ArrowRight => keys::RIGHT,
        Code::KeyX => keys::A,
        Code::KeyZ => keys::B,
        Code::KeyA => keys::L,
        Code::KeyS => keys::R,
        Code::Enter => keys::START,
        Code::Backspace => keys::SELECT,
        _ => return None,
    })
}

fn fkey_slot(code: &Code) -> Option<u8> {
    Some(match code {
        Code::F1 => 1,
        Code::F2 => 2,
        Code::F3 => 3,
        Code::F4 => 4,
        Code::F5 => 5,
        Code::F6 => 6,
        Code::F7 => 7,
        Code::F8 => 8,
        Code::F9 => 9,
        _ => return None,
    })
}

/// Shared session state handed to the view; compared by pointer.
#[derive(Clone)]
pub struct SharedRef(Arc<Shared>);

impl PartialEq for SharedRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[component]
pub fn Player() -> Element {
    let state = use_context::<AppState>();
    let id = *state.playing.read();
    let shared = state.session.read().as_ref().map(|s| SharedRef(s.shared.clone()));
    match (id, shared) {
        (Some(id), Some(shared)) => rsx! { PlayerView { key: "{id}", id, shared } },
        _ => rsx! {},
    }
}

#[component]
fn PlayerView(id: usize, shared: SharedRef) -> Element {
    let state = use_context::<AppState>();
    let shared = shared.0;
    let game = state.game(id).unwrap_or_else(|| state.session.peek().as_ref().unwrap().game.clone());

    let mut paused = use_signal(|| false);
    let mut panel = use_signal(|| Panel::Home);
    let mut fps = use_signal(|| 0u32);
    let mut turbo = use_signal(|| false);
    let mut pad = use_signal(|| None::<String>);
    let mut states_ver = use_signal(|| 0u64);

    use_effect(move || {
        document::eval("window.__gba.start()");
    });
    use_drop(|| {
        document::eval("window.__gba.stop()");
    });

    // Poll the emulation thread for stats, toasts and gamepad menu presses.
    let sh = shared.clone();
    use_future(move || {
        let sh = sh.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let f = sh.fps.load(Ordering::Relaxed);
                if *fps.peek() != f {
                    fps.set(f);
                }
                let msgs = std::mem::take(&mut *sh.toasts.lock().unwrap());
                for m in msgs {
                    state.toast(m);
                }
                let v = sh.states_version.load(Ordering::Relaxed);
                if *states_ver.peek() != v {
                    states_ver.set(v);
                }
                let name = sh.gamepad.lock().unwrap().clone();
                if *pad.peek() != name {
                    pad.set(name);
                }
                if sh.menu_requested.swap(false, Ordering::Relaxed) {
                    let p = !*paused.peek();
                    sh.paused.store(p, Ordering::Relaxed);
                    sh.keyboard.store(0, Ordering::Relaxed);
                    paused.set(p);
                    panel.set(Panel::Home);
                }
            }
        }
    });

    let settings = state.store.read().settings.clone();
    let filter = settings.video_filter.clone();
    let integer = settings.integer_scale;
    // Re-fit the canvas whenever scaling-related settings change.
    let filter_dep = filter.clone();
    use_effect(use_reactive!(|(integer, filter_dep)| {
        let _ = (integer, filter_dep);
        document::eval("setTimeout(() => window.__gba.fit(), 0)");
    }));

    let sh_pause = shared.clone();
    let set_paused = use_callback(move |p: bool| {
        sh_pause.paused.store(p, Ordering::Relaxed);
        sh_pause.keyboard.store(0, Ordering::Relaxed);
        paused.set(p);
        panel.set(Panel::Home);
        if !p {
            document::eval("window.__gba.focus()");
        }
    });

    let send = move |cmd: Cmd| {
        if let Some(s) = state.session.peek().as_ref() {
            s.send(cmd);
        }
    };

    let sh_down = shared.clone();
    let onkeydown = move |e: KeyboardEvent| {
        let code = e.code();
        let shift = e.modifiers().shift();
        let is_paused = *paused.peek();
        if let Some(bit) = key_bit(&code) {
            e.prevent_default();
            if !is_paused {
                sh_down.keyboard.fetch_or(bit, Ordering::Relaxed);
            }
            return;
        }
        if e.is_auto_repeating() {
            return;
        }
        match code {
            Code::Escape => {
                e.prevent_default();
                if is_paused && *panel.peek() != Panel::Home {
                    panel.set(Panel::Home);
                } else {
                    set_paused.call(!is_paused);
                }
            }
            Code::Tab => {
                e.prevent_default();
                sh_down.turbo.store(true, Ordering::Relaxed);
                turbo.set(true);
            }
            Code::F12 => {
                e.prevent_default();
                send(Cmd::Screenshot);
            }
            ref c => {
                if let Some(slot) = fkey_slot(c) {
                    e.prevent_default();
                    send(if shift { Cmd::SaveState(slot) } else { Cmd::LoadState(slot) });
                }
            }
        }
    };
    let sh_up = shared.clone();
    let onkeyup = move |e: KeyboardEvent| {
        let code = e.code();
        if let Some(bit) = key_bit(&code) {
            sh_up.keyboard.fetch_and(!bit, Ordering::Relaxed);
        } else if code == Code::Tab {
            sh_up.turbo.store(false, Ordering::Relaxed);
            turbo.set(false);
        }
    };
    let sh_blur = shared.clone();

    let title = game.title.clone();
    let fps_text = format!("{:.1}", fps() as f64 / 10.0);
    let is_paused = paused();
    let current_panel = panel();
    let ver = states_ver();
    let slot_info: Vec<(u8, Option<u64>)> = (1..=STATE_SLOTS)
        .map(|s| {
            let m = std::fs::metadata(game.state_path(s)).ok();
            (s, m.map(|m| crate::library::mtime_secs(&m)))
        })
        .collect();
    let volume = settings.volume;
    let sh_vol = shared.clone();

    let menu_btn = |p: Panel, icon: &'static str, label: &'static str| {
        rsx! {
            button {
                class: if current_panel == p { "menu-btn on" } else { "menu-btn" },
                onclick: move |_| panel.set(p),
                span { "{icon}" } span { "{label}" }
            }
        }
    };
    let seg = |v: &'static str, label: &'static str| {
        let on = filter == v;
        rsx! {
            button {
                class: if on { "on" } else { "" },
                onclick: move |_| state.update_settings(|s| s.video_filter = v.into()),
                "{label}"
            }
        }
    };

    rsx! {
        div {
            id: "player-root",
            class: "player filter-{filter}",
            tabindex: "0",
            onkeydown,
            onkeyup,
            onblur: move |_| sh_blur.keyboard.store(0, Ordering::Relaxed),

            div { class: "player-stage", id: "gba-stage",
                div { class: "ambient", style: "background-image: url('/art/title/{id}')" }
                div { class: "screen-wrap",
                    canvas { id: "gba-screen", width: "240", height: "160", "data-integer": "{integer}" }
                    div { id: "gba-overlay" }
                }

                div { class: if is_paused { "hud show" } else { "hud" },
                    span { class: "chip", "🎮 {title}" }
                    if let Some(p) = pad() { span { class: "chip", "🕹 {p}" } }
                    if turbo() { span { class: "chip turbo", "⏩ Fast-forward" } }
                    span { class: "spacer" }
                    span { class: "chip", "Esc · menu" }
                }
                if settings.show_fps && !is_paused {
                    div { class: "fps", "{fps_text} fps" }
                }

                if is_paused {
                    div { class: "pause",
                        div { class: "pause-menu",
                            div { class: "game", "{title}" }
                            div { class: "muted", "Paused" }
                            button { class: "menu-btn", onclick: move |_| set_paused.call(false), span { "▶" } span { "Resume" } kbd { class: "k", "Esc" } }
                            {menu_btn(Panel::Save, "💾", "Save state")}
                            {menu_btn(Panel::Load, "📂", "Load state")}
                            {menu_btn(Panel::Display, "🖥", "Display & sound")}
                            {menu_btn(Panel::Controls, "⌨️", "Controls")}
                            button { class: "menu-btn", onclick: move |_| send(Cmd::Screenshot), span { "📸" } span { "Screenshot" } }
                            button { class: "menu-btn", onclick: move |_| { send(Cmd::Reset); set_paused.call(false); }, span { "⟲" } span { "Reset" } }
                            div { style: "flex:1" }
                            button { class: "menu-btn", onclick: move |_| state.quit_game(), span { "⏏" } span { "Quit to library" } }
                        }
                        div { class: "pause-panel",
                            match current_panel {
                                Panel::Home => rsx! {
                                    h3 { "Paused" }
                                    p { style: "color: var(--muted); max-width: 520px",
                                        "Your progress is saved automatically when you quit, and you can pick up right where you left off from the library. "
                                        "Use save states for checkpoints you want to keep."
                                    }
                                    div { class: "h3", "Quick keys" }
                                    ControlsTable {}
                                },
                                Panel::Save | Panel::Load => {
                                    let saving = current_panel == Panel::Save;
                                    rsx! {
                                        h3 { if saving { "Save to a slot" } else { "Load a slot" } }
                                        div { class: "slots",
                                            for (slot, mtime) in slot_info.clone() {
                                                button {
                                                    key: "{slot}",
                                                    class: "slot",
                                                    disabled: !saving && mtime.is_none(),
                                                    onclick: move |_| {
                                                        if saving {
                                                            send(Cmd::SaveState(slot));
                                                        } else {
                                                            send(Cmd::LoadState(slot));
                                                            set_paused.call(false);
                                                        }
                                                    },
                                                    div { class: "thumb",
                                                        if let Some(m) = mtime {
                                                            img { src: "/art/state/{id}/{slot}?v={m}-{ver}", alt: "", draggable: "false" }
                                                        } else {
                                                            "Empty"
                                                        }
                                                    }
                                                    div { class: "cap",
                                                        b { "Slot {slot}" }
                                                        span { if let Some(m) = mtime { "{ago(m)}" } else { "—" } }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                                Panel::Display => rsx! {
                                    h3 { "Display & sound" }
                                    div { class: "field",
                                        div { class: "lbl", "Video filter" }
                                        div { class: "seg",
                                            {seg("pixel", "Pixel")}
                                            {seg("smooth", "Smooth")}
                                            {seg("lcd", "LCD")}
                                            {seg("crt", "CRT")}
                                        }
                                    }
                                    div { class: "field",
                                        div { class: "lbl", "Integer scaling" }
                                        button {
                                            class: if integer { "switch on" } else { "switch" },
                                            onclick: move |_| state.update_settings(|s| s.integer_scale = !s.integer_scale),
                                        }
                                    }
                                    div { class: "field",
                                        div { class: "lbl", "Show FPS" }
                                        button {
                                            class: if settings.show_fps { "switch on" } else { "switch" },
                                            onclick: move |_| state.update_settings(|s| s.show_fps = !s.show_fps),
                                        }
                                    }
                                    div { class: "field",
                                        div { class: "lbl", "Volume" small { "{volume}%" } }
                                        input {
                                            class: "range",
                                            r#type: "range", min: "0", max: "100",
                                            value: "{volume}",
                                            oninput: move |e| {
                                                let v: u32 = e.value().parse().unwrap_or(80);
                                                sh_vol.volume.store(v, Ordering::Relaxed);
                                                state.update_settings(|s| s.volume = v);
                                            },
                                        }
                                    }
                                },
                                Panel::Controls => rsx! {
                                    h3 { "Controls" }
                                    ControlsTable {}
                                    p { style: "color: var(--muted); margin-top: 18px",
                                        "Gamepads are detected automatically (Xbox, PlayStation, Switch Pro, 8BitDo…). "
                                        "Right trigger fast-forwards; Home or Select+Start opens this menu."
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}
