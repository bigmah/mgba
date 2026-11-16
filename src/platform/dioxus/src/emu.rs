//! The emulation session: an mGBA core running on its own thread, with audio
//! output (cpal), gamepad input (gilrs) and a shared framebuffer that the
//! webview pulls from via the `/emu/frame` asset route.

use crate::ffi::{keys, Core};
use crate::library::{resume_path, screenshot_dir, Game};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dioxus::desktop::wry::{http::Response, RequestAsyncResponder};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub enum Cmd {
    SaveState(u8),
    LoadState(u8),
    Reset,
    Screenshot,
    Quit { save_resume: bool },
}

#[derive(Default)]
pub struct FrameBuf {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub seq: u64,
}

#[derive(Default)]
pub struct Shared {
    pub frame: Mutex<FrameBuf>,
    /// Keys held on the keyboard (GBA key bits), written by the UI thread.
    pub keyboard: AtomicU32,
    pub paused: AtomicBool,
    pub turbo: AtomicBool,
    /// 0..=100
    pub volume: AtomicU32,
    /// Measured frames per second × 10.
    pub fps: AtomicU32,
    /// Set by the emulation thread when the gamepad's Home/Guide button is pressed.
    pub menu_requested: AtomicBool,
    pub gamepad: Mutex<Option<String>>,
    /// Short user-facing messages ("Saved slot 1") drained by the UI.
    pub toasts: Mutex<Vec<String>>,
    /// Bumped whenever a state slot is written so thumbnails refresh.
    pub states_version: AtomicU64,
}

impl Shared {
    fn toast(&self, msg: impl Into<String>) {
        self.toasts.lock().unwrap().push(msg.into());
    }
}

/// The session currently feeding `/emu/frame` (if any).
static CURRENT: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

pub struct Session {
    pub shared: Arc<Shared>,
    pub game: Game,
    pub started: Instant,
    tx: mpsc::Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub fn start(game: Game, resume_from: Option<PathBuf>, volume: u32) -> Result<Session, String> {
        let shared = Arc::new(Shared::default());
        shared.volume.store(volume, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let thread_shared = shared.clone();
        let thread_game = game.clone();
        let thread = std::thread::Builder::new()
            .name("mgba-core".into())
            .spawn(move || run(thread_game, resume_from, thread_shared, rx, ready_tx))
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv()
            .map_err(|_| "emulation thread crashed".to_string())??;
        *CURRENT.lock().unwrap() = Some(shared.clone());
        Ok(Session {
            shared,
            game,
            started: Instant::now(),
            tx,
            thread: Some(thread),
        })
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit { save_resume: true });
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let mut cur = CURRENT.lock().unwrap();
        if cur.as_ref().is_some_and(|s| Arc::ptr_eq(s, &self.shared)) {
            *cur = None;
        }
    }
}

/// Asset route: `/emu/frame?s=<last seq>`. Body = [u32 w][u32 h][u64 seq][RGBA...].
pub fn serve_frame(uri_query: Option<&str>, responder: RequestAsyncResponder) {
    let since: u64 = uri_query
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("s=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let shared = CURRENT.lock().unwrap().clone();
    let body = shared.and_then(|s| {
        let f = s.frame.lock().unwrap();
        if f.seq == since || f.rgba.is_empty() {
            return None;
        }
        let mut body = Vec::with_capacity(16 + f.rgba.len());
        body.extend_from_slice(&f.width.to_le_bytes());
        body.extend_from_slice(&f.height.to_le_bytes());
        body.extend_from_slice(&f.seq.to_le_bytes());
        body.extend_from_slice(&f.rgba);
        Some(body)
    });
    let resp = match body {
        Some(body) => Response::builder()
            .status(200)
            .header("Content-Type", "application/octet-stream")
            .header("Cache-Control", "no-store")
            .body(body),
        None => Response::builder()
            .status(204)
            .header("Cache-Control", "no-store")
            .body(Vec::new()),
    };
    responder.respond(resp.unwrap());
}

fn run(
    game: Game,
    resume_from: Option<PathBuf>,
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Cmd>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let Some(mut core) = Core::open(&game.path, Some(&game.save_path())) else {
        let _ = ready.send(Err(format!("mGBA could not load {}", game.path.display())));
        return;
    };
    if let Some(state) = resume_from {
        if core.load_state(&state) {
            shared.toast("Resumed where you left off");
        }
    }
    let _ = ready.send(Ok(()));

    let fps = core.fps();
    let in_rate = core.sample_rate();
    let mut audio = Audio::new(shared.clone());
    if audio.is_none() {
        shared.toast("No audio device — running silently");
    }
    let mut pads = Pads::new(shared.clone());

    let frame_time = Duration::from_secs_f64(1.0 / fps);
    let mut next = Instant::now();
    let mut samples = vec![0i16; 8192];
    let mut local = Vec::new();
    let mut fps_window = (Instant::now(), 0u32);

    loop {
        // --- commands -----------------------------------------------------
        let paused = shared.paused.load(Ordering::Relaxed);
        let cmd = if paused {
            rx.recv_timeout(Duration::from_millis(16)).ok()
        } else {
            rx.try_recv().ok()
        };
        if let Some(cmd) = cmd {
            match cmd {
                Cmd::Quit { save_resume } => {
                    if save_resume {
                        core.save_state(&resume_path(&game));
                    }
                    break;
                }
                Cmd::SaveState(slot) => {
                    if core.save_state(&game.state_path(slot)) {
                        shared.states_version.fetch_add(1, Ordering::Relaxed);
                        shared.toast(format!("Saved to slot {slot}"));
                    } else {
                        shared.toast(format!("Couldn't save slot {slot}"));
                    }
                }
                Cmd::LoadState(slot) => {
                    let path = game.state_path(slot);
                    if !path.exists() {
                        shared.toast(format!("Slot {slot} is empty"));
                    } else if core.load_state(&path) {
                        shared.toast(format!("Loaded slot {slot}"));
                    } else {
                        shared.toast(format!("Couldn't load slot {slot}"));
                    }
                }
                Cmd::Reset => {
                    core.reset();
                    shared.toast("Reset");
                }
                Cmd::Screenshot => {
                    let path = screenshot_dir().join(format!(
                        "{} {}.png",
                        game.stem,
                        crate::library::now_secs()
                    ));
                    if core.screenshot(&path) {
                        shared.toast("Screenshot saved");
                    }
                }
            }
            continue;
        }

        pads.poll();
        if paused {
            next = Instant::now();
            if let Some(a) = audio.as_mut() {
                a.clear();
            }
            continue;
        }

        // --- emulate one frame ---------------------------------------------
        let held = shared.keyboard.load(Ordering::Relaxed) | pads.keys;
        core.set_keys(held);
        core.run_frame();

        let (w, h) = core.frame_rgba(&mut local);
        {
            let mut f = shared.frame.lock().unwrap();
            std::mem::swap(&mut f.rgba, &mut local);
            f.width = w;
            f.height = h;
            f.seq += 1;
        }

        let turbo = shared.turbo.load(Ordering::Relaxed) || pads.turbo;
        let n = core.read_audio(&mut samples);
        if let Some(a) = audio.as_mut() {
            if turbo {
                a.clear();
            } else {
                a.push(&samples[..n * 2], in_rate);
            }
        }

        // --- stats & pacing --------------------------------------------------
        fps_window.1 += 1;
        let elapsed = fps_window.0.elapsed();
        if elapsed >= Duration::from_millis(500) {
            let measured = fps_window.1 as f64 / elapsed.as_secs_f64();
            shared.fps.store((measured * 10.0).round() as u32, Ordering::Relaxed);
            fps_window = (Instant::now(), 0);
        }

        if turbo {
            next = Instant::now();
            continue;
        }
        next += frame_time;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(100) {
            next = now; // fell far behind (e.g. window dragged); don't try to catch up
        }
    }
    // `core` drops here: ROM unloaded and the battery save flushed.
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

struct Audio {
    _stream: cpal::Stream,
    queue: Arc<Mutex<VecDeque<f32>>>,
    out_rate: u32,
    shared: Arc<Shared>,
    pos: f64,
    last: [f32; 2],
    scratch: Vec<f32>,
}

impl Audio {
    fn new(shared: Arc<Shared>) -> Option<Audio> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let queue = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(48000)));

        let q = queue.clone();
        let fill = move |out: &mut dyn FnMut(usize, f32), frames: usize| {
            let mut q = q.lock().unwrap();
            for i in 0..frames {
                let (l, r) = if q.len() >= 2 {
                    (q.pop_front().unwrap(), q.pop_front().unwrap())
                } else {
                    (0.0, 0.0)
                };
                for c in 0..channels {
                    out(i * channels + c, match c {
                        0 => l,
                        1 => r,
                        _ => (l + r) * 0.5,
                    });
                }
            }
        };
        let err = |e| eprintln!("audio stream error: {e}");
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let frames = data.len() / channels;
                    fill(&mut |i, v| data[i] = v, frames)
                },
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let frames = data.len() / channels;
                    fill(&mut |i, v| data[i] = (v * 32767.0) as i16, frames)
                },
                err,
                None,
            ),
            _ => return None,
        }
        .ok()?;
        stream.play().ok()?;
        Some(Audio {
            _stream: stream,
            queue,
            out_rate: config.sample_rate.0,
            shared,
            pos: 0.0,
            last: [0.0; 2],
            scratch: Vec::with_capacity(4096),
        })
    }

    fn clear(&mut self) {
        self.queue.lock().unwrap().clear();
    }

    /// Resample interleaved stereo i16 at `in_rate` to the device rate with
    /// linear interpolation, nudging the ratio by ±0.5% to hold ~60 ms of buffer.
    fn push(&mut self, input: &[i16], in_rate: u32) {
        let frames = input.len() / 2;
        if frames == 0 {
            return;
        }
        let target = (self.out_rate as f64 * 0.06) * 2.0;
        let fill = self.queue.lock().unwrap().len() as f64;
        let adjust = ((fill - target) / target).clamp(-1.0, 1.0) * 0.005;
        let step = in_rate as f64 / self.out_rate as f64 * (1.0 + adjust);

        let vol = self.shared.volume.load(Ordering::Relaxed).min(100) as f32 / 100.0;
        let gain = vol * vol / 32768.0;
        let sample = |i: usize, c: usize| input[i * 2 + c] as f32 * gain;

        self.scratch.clear();
        while self.pos < frames as f64 {
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            for c in 0..2 {
                let a = if i == 0 { self.last[c] } else { sample(i - 1, c) };
                let b = sample(i, c);
                self.scratch.push(a + (b - a) * t);
            }
            self.pos += step;
        }
        self.pos -= frames as f64;
        self.last = [sample(frames - 1, 0), sample(frames - 1, 1)];

        let mut q = self.queue.lock().unwrap();
        q.extend(self.scratch.iter().copied());
        // Never let latency balloon (e.g. after a stall): drop the oldest audio.
        let max = (target * 4.0) as usize;
        if q.len() > max {
            let excess = (q.len() - target as usize) & !1;
            q.drain(..excess);
        }
    }
}

// ---------------------------------------------------------------------------
// Gamepads
// ---------------------------------------------------------------------------

struct Pads {
    gilrs: Option<gilrs::Gilrs>,
    shared: Arc<Shared>,
    keys: u32,
    turbo: bool,
    guide_was_down: bool,
}

impl Pads {
    fn new(shared: Arc<Shared>) -> Pads {
        let gilrs = gilrs::Gilrs::new().ok();
        Pads {
            gilrs,
            shared,
            keys: 0,
            turbo: false,
            guide_was_down: false,
        }
    }

    fn poll(&mut self) {
        use gilrs::{Axis, Button};
        let Some(g) = self.gilrs.as_mut() else { return };
        while g.next_event().is_some() {}

        let mut keys = 0;
        let mut turbo = false;
        let mut guide = false;
        let mut name = None;
        for (_, pad) in g.gamepads() {
            name.get_or_insert_with(|| pad.name().to_string());
            let map = [
                (Button::East, keys::A),
                (Button::South, keys::B),
                (Button::North, keys::A),
                (Button::West, keys::B),
                (Button::Select, keys::SELECT),
                (Button::Start, keys::START),
                (Button::DPadUp, keys::UP),
                (Button::DPadDown, keys::DOWN),
                (Button::DPadLeft, keys::LEFT),
                (Button::DPadRight, keys::RIGHT),
                (Button::LeftTrigger, keys::L),
                (Button::RightTrigger, keys::R),
            ];
            for (button, bit) in map {
                if pad.is_pressed(button) {
                    keys |= bit;
                }
            }
            let (x, y) = (pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY));
            if x > 0.5 {
                keys |= keys::RIGHT;
            }
            if x < -0.5 {
                keys |= keys::LEFT;
            }
            if y > 0.5 {
                keys |= keys::UP;
            }
            if y < -0.5 {
                keys |= keys::DOWN;
            }
            turbo |= pad.is_pressed(Button::RightTrigger2);
            guide |= pad.is_pressed(Button::Mode)
                || (pad.is_pressed(Button::Select) && pad.is_pressed(Button::Start));
        }
        if guide && !self.guide_was_down {
            self.shared.menu_requested.store(true, Ordering::Relaxed);
        }
        self.guide_was_down = guide;
        self.keys = keys;
        self.turbo = turbo;
        let mut slot = self.shared.gamepad.lock().unwrap();
        if *slot != name {
            *slot = name;
        }
    }
}
