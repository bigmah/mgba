# mGBA × Dioxus

A library-first desktop frontend for mGBA, written in Rust with
[Dioxus](https://dioxuslabs.com). It embeds this fork's `libmgba` directly, so
games run in-process with no external emulator.

## Features

- **Browsable library.** Box-art grid built from your ROM folder, with No-Intro
  name parsing (regions, languages, revisions, translation tags) and a
  `The Legend of Zelda`-style title cleanup.
- **Artwork.** Box art, title screens and in-game snaps come from
  [libretro-thumbnails](https://github.com/libretro-thumbnails) and are cached in
  `~/Library/Application Support/mgba-dioxus/art` (or your platform's data dir).
- **Finding games.** Instant search (`/` or `⌘F`), sections (Continue Playing,
  Favorites, Recently Played, With Saves, Translations), regions, auto-detected
  series, sort modes, an A–Z rail, adjustable cover size, and 🎲 *surprise me*.
- **Details drawer.** ROM header info, No-Intro hashes (read from `<rom>.txt`),
  play stats, and save-state thumbnails.
- **Continue where you left off.** Quitting a game writes an automatic resume
  state. The home hero and the *Jump back in* shelf show where you stopped.
- **Saves and states.** Saves and states are compatible with mGBA:
  `<rom>.sav` battery saves and `<rom>.ss1`–`.ss9` slots sit next to the ROM.
  States are PNG-wrapped with an embedded screenshot, so states made by the
  official mGBA load here, and the reverse also works.
- **Player.** Pixel, Smooth, LCD-grid or CRT filters, integer scaling, a
  fast-forward hold, a pause menu with visual save/load slots, screenshots and
  volume. Gamepads are supported through gilrs.

## Building

Requirements: Rust (stable), CMake, and a C compiler.

```sh
cd src/platform/dioxus
cargo run --release -- /path/to/roms
```

`build.rs` builds `libmgba` from this repository, using `LIBMGBA_ONLY` with
the new `LIBMGBA_BUNDLED_PNG` option to get the vendored zlib and libpng. It
then compiles a small C shim (`shim/mgba_shim.c`) using the *exact* compile
definitions CMake used for the core, so the Rust side never has to mirror
mGBA's feature-dependent struct layouts.

The ROM folder is resolved in this order: CLI argument, `$MGBA_ROMS`, the
folder saved in settings, then `~/dev/gba/roms` and `~/roms`. You can change it
in-app under ⚙︎ Settings.

## Controls

| GBA | Keyboard | Gamepad |
| --- | --- | --- |
| D-Pad | Arrow keys | D-Pad / left stick |
| A / B | X / Z | East / South |
| L / R | A / S | LB / RB |
| Start / Select | Enter / Backspace | Start / Select |
| Fast-forward (hold) | Tab | RT |
| Pause menu | Esc | Home, or Select+Start |
| Load / save state 1–9 | F1–F9 / ⇧F1–F9 | — |
| Screenshot | F12 | — |

## Architecture

```
┌──────────── Dioxus webview ────────────┐      ┌───── emulation thread ─────┐
│ Library / Details / Settings (Rust UI) │      │ libmgba core (C, via shim) │
│ <canvas> ◀── fetch /emu/frame ─────────┼──────┤ shared RGBA framebuffer    │
│ <img> ◀───── /art/... (cached art)     │      │ cpal audio (resampled)     │
│ keyboard ───▶ atomic key bits ─────────┼─────▶│ gilrs gamepads             │
└────────────────────────────────────────┘      └────────────────────────────┘
```

- `src/ffi.rs`: a safe wrapper over the C shim.
- `src/emu.rs`: the session thread, frame pacing, audio resampling with dynamic
  rate control, gamepads, and the `/emu/frame` route.
- `src/library.rs`: scanning, name parsing, ROM headers, and persistence.
- `src/art.rs`: the libretro thumbnail fetcher with its disk cache.
- `src/ui_library.rs`, `src/ui_player.rs`: the UI.

## Diagnostics

```sh
cargo run -- --selftest rom.gba [state.ss1]   # headless core test
cargo run -- --list /path/to/roms             # show how names are parsed
MGBA_SELFCHECK=<game id> cargo run -- roms/   # drive the real UI end to end
MGBA_LOG=1 cargo run                          # verbose mGBA core logging
```
