//! The library browser: sidebar, search, hero / "jump back in" shelf, the
//! box-art grid with an A–Z rail, the game details drawer, and settings.

use crate::library::{self, now_secs, Game};
use crate::{art, reveal_in_finder, AppState};
use dioxus::prelude::*;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, PartialEq)]
enum Section {
    All,
    Continue,
    Favorites,
    Recent,
    Saves,
    Translations,
    Region(&'static str),
    Series(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Alpha,
    Recent,
    Playtime,
    Size,
}

pub fn ago(ts: u64) -> String {
    if ts == 0 {
        return "never".into();
    }
    let d = now_secs().saturating_sub(ts);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", d / 60),
        3600..=86399 => format!("{}h ago", d / 3600),
        86400..=604799 => format!("{}d ago", d / 86400),
        _ => format!("{}w ago", d / 604800),
    }
}

pub fn duration(secs: u64) -> String {
    match secs {
        0 => "—".into(),
        1..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

fn hue(s: &str) -> u32 {
    s.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360
}

fn matches(g: &Game, tokens: &[String]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {}",
        g.title, g.stem, g.header.internal_title, g.header.game_code
    )
    .to_lowercase();
    tokens.iter().all(|t| hay.contains(t.as_str()))
}

#[component]
pub fn Library() -> Element {
    let state = use_context::<AppState>();
    let mut search = use_signal(String::new);
    let mut section = use_signal(|| Section::All);
    let mut sort = use_signal(|| Sort::Alpha);
    let mut settings_open = use_signal(|| false);

    let games = state.games.read().clone();
    // Disk-derived sets, recomputed when a game exits or the library changes.
    let resumable = use_memo(move || {
        state.tick.read();
        state
            .games
            .read()
            .iter()
            .filter(|g| library::resume_path(g).exists())
            .map(|g| g.id)
            .collect::<HashSet<_>>()
    });
    let with_saves = use_memo(move || {
        state.tick.read();
        state
            .games
            .read()
            .iter()
            .filter(|g| g.has_save() || !g.state_slots().is_empty())
            .map(|g| g.id)
            .collect::<HashSet<_>>()
    });
    let featured = use_hook(|| now_secs() as usize);

    let store = state.store.read();
    let resumable = resumable.read();
    let with_saves = with_saves.read();

    // ---- sidebar data ----------------------------------------------------
    let count_by = |f: &dyn Fn(&Game) -> bool| games.iter().filter(|g| f(g)).count();
    let n_fav = count_by(&|g| store.favorites.contains(&g.stem));
    let n_recent = count_by(&|g| store.stats(g).last_played > 0);
    let n_tr = count_by(&|g| g.is_translation());
    let regions: Vec<(&'static str, usize)> = ["USA", "Europe", "Japan", "World"]
        .into_iter()
        .map(|r| (r, count_by(&|g| g.regions.iter().any(|x| x == r))))
        .filter(|(_, n)| *n > 0)
        .collect();
    let mut series: Vec<(String, usize)> = games
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut m, g| {
            *m.entry(g.series.clone()).or_default() += 1;
            m
        })
        .into_iter()
        .filter(|(_, n)| *n >= 3)
        .collect();
    series.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    series.truncate(14);

    // ---- filtered list -----------------------------------------------------
    let tokens: Vec<String> = search
        .read()
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let sec = section.read().clone();
    let mut list: Vec<&Game> = games
        .iter()
        .filter(|g| match &sec {
            Section::All => true,
            Section::Continue => resumable.contains(&g.id),
            Section::Favorites => store.favorites.contains(&g.stem),
            Section::Recent => store.stats(g).last_played > 0,
            Section::Saves => with_saves.contains(&g.id),
            Section::Translations => g.is_translation(),
            Section::Region(r) => g.regions.iter().any(|x| x == r),
            Section::Series(s) => &g.series == s,
        })
        .filter(|g| matches(g, &tokens))
        .collect();
    let effective_sort = match (&sec, *sort.read()) {
        (Section::Recent | Section::Continue, Sort::Alpha) => Sort::Recent,
        (_, s) => s,
    };
    match effective_sort {
        Sort::Alpha => {}
        Sort::Recent => list.sort_by_key(|g| std::cmp::Reverse(store.stats(g).last_played)),
        Sort::Playtime => list.sort_by_key(|g| std::cmp::Reverse(store.stats(g).seconds)),
        Sort::Size => list.sort_by_key(|g| std::cmp::Reverse(g.size)),
    }
    let alpha = effective_sort == Sort::Alpha;
    let mut letters_present = HashSet::new();
    let cards: Vec<(usize, Option<char>, bool, bool)> = list
        .iter()
        .map(|g| {
            let anchor = (alpha && letters_present.insert(g.letter)).then_some(g.letter);
            (
                g.id,
                anchor,
                store.favorites.contains(&g.stem),
                with_saves.contains(&g.id),
            )
        })
        .collect();

    // ---- hero ------------------------------------------------------------
    let show_hero = sec == Section::All && tokens.is_empty() && !games.is_empty();
    let mut recent: Vec<&Game> = games
        .iter()
        .filter(|g| resumable.contains(&g.id) && store.stats(g).last_played > 0)
        .collect();
    recent.sort_by_key(|g| std::cmp::Reverse(store.stats(g).last_played));
    recent.truncate(10);
    let hero_game: Option<Game> = recent
        .first()
        .map(|g| (*g).clone())
        .or_else(|| games.get(featured % games.len().max(1)).cloned());
    let hero_is_resume = !recent.is_empty();
    let shelf: Vec<(usize, String, u64, u64)> = recent
        .iter()
        .skip(1)
        .map(|g| {
            let s = store.stats(g);
            (g.id, g.title.clone(), s.last_played, s.seconds)
        })
        .collect();

    let card_size = store.settings.card_size;
    let section_title = match &sec {
        Section::All => "All Games".to_string(),
        Section::Continue => "Continue Playing".into(),
        Section::Favorites => "Favorites".into(),
        Section::Recent => "Recently Played".into(),
        Section::Saves => "With Saves".into(),
        Section::Translations => "Translations & Hacks".into(),
        Section::Region(r) => format!("{} {r}", library::region_flag(r).unwrap_or("")),
        Section::Series(s) => s.clone(),
    };
    let total = games.len();
    let shown = list.len();
    let n_continue = resumable.len();
    let n_saves = with_saves.len();
    let has_dir = state.roms_dir.read().is_some();
    let selected = *state.selected.read();
    drop(store);
    drop(resumable);
    drop(with_saves);

    let nav = move |s: Section| {
        move |_| {
            section.set(s.clone());
            document::eval("window.__lib.top()");
        }
    };
    let is = |s: Section| if sec == s { "nav-item active" } else { "nav-item" };

    rsx! {
        div {
            class: "app",
            tabindex: "-1",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            onkeydown: move |e: KeyboardEvent| {
                if state.playing.peek().is_some() { return; }
                let k = e.key().to_string();
                if k == "/" || (k == "f" && e.modifiers().meta()) {
                    e.prevent_default();
                    document::eval("window.__lib.focusSearch()");
                } else if k == "Escape" {
                    if settings_open() { settings_open.set(false) }
                    else if state.selected.peek().is_some() { state.selected.clone().set(None) }
                    else { search.set(String::new()) }
                }
            },

            // ---------------------------------------------------- top bar
            header { class: "topbar",
                div { class: "brand",
                    div { class: "brand-logo", "▶" }
                    div { "mGBA" small { "Library" } }
                }
                div { class: "search",
                    span { class: "icon", "🔍" }
                    input {
                        id: "search",
                        placeholder: "Search {total} games…",
                        value: "{search}",
                        spellcheck: "false",
                        autocomplete: "off",
                        oninput: move |e| search.set(e.value()),
                    }
                    kbd { "/" }
                }
                div { class: "top-actions",
                    select {
                        class: "select",
                        onchange: move |e| sort.set(match e.value().as_str() {
                            "recent" => Sort::Recent,
                            "playtime" => Sort::Playtime,
                            "size" => Sort::Size,
                            _ => Sort::Alpha,
                        }),
                        option { value: "alpha", "A → Z" }
                        option { value: "recent", "Recently played" }
                        option { value: "playtime", "Most played" }
                        option { value: "size", "Largest" }
                    }
                    input {
                        class: "range",
                        r#type: "range",
                        min: "120",
                        max: "280",
                        value: "{card_size}",
                        title: "Cover size",
                        oninput: move |e| {
                            let v = e.value().parse().unwrap_or(170);
                            state.update_settings(|s| s.card_size = v);
                        },
                    }
                    button {
                        class: "btn-ghost",
                        title: "Surprise me",
                        disabled: total == 0,
                        onclick: move |_| {
                            let n = state.games.peek().len();
                            if n > 0 {
                                let pick = (crate::now_nanos() % n as u64) as usize;
                                state.selected.clone().set(Some(pick));
                            }
                        },
                        "🎲"
                    }
                    button { class: "btn-ghost", title: "Settings", onclick: move |_| settings_open.set(true), "⚙︎" }
                }
            }

            // ---------------------------------------------------- sidebar
            nav { class: "sidebar",
                div { class: "nav-title", "Library" }
                button { class: is(Section::All), onclick: nav(Section::All),
                    span { class: "ico", "🎮" } span { class: "label", "All Games" } span { class: "count", "{total}" } }
                button { class: is(Section::Continue), onclick: nav(Section::Continue),
                    span { class: "ico", "⏯" } span { class: "label", "Continue Playing" } span { class: "count", "{n_continue}" } }
                button { class: is(Section::Favorites), onclick: nav(Section::Favorites),
                    span { class: "ico", "★" } span { class: "label", "Favorites" } span { class: "count", "{n_fav}" } }
                button { class: is(Section::Recent), onclick: nav(Section::Recent),
                    span { class: "ico", "🕘" } span { class: "label", "Recently Played" } span { class: "count", "{n_recent}" } }
                button { class: is(Section::Saves), onclick: nav(Section::Saves),
                    span { class: "ico", "💾" } span { class: "label", "With Saves" } span { class: "count", "{n_saves}" } }
                if n_tr > 0 {
                    button { class: is(Section::Translations), onclick: nav(Section::Translations),
                        span { class: "ico", "🌐" } span { class: "label", "Translations" } span { class: "count", "{n_tr}" } }
                }
                div { class: "nav-title", "Regions" }
                for (r, n) in regions {
                    button { key: "{r}", class: is(Section::Region(r)), onclick: nav(Section::Region(r)),
                        span { class: "ico", {library::region_flag(r).unwrap_or("·")} }
                        span { class: "label", "{r}" }
                        span { class: "count", "{n}" }
                    }
                }
                if !series.is_empty() {
                    div { class: "nav-title", "Series" }
                }
                for (s, n) in series {
                    button { key: "{s}", class: is(Section::Series(s.clone())), onclick: nav(Section::Series(s.clone())),
                        span { class: "ico", "◆" }
                        span { class: "label", "{s}" }
                        span { class: "count", "{n}" }
                    }
                }
            }

            // ---------------------------------------------------- main
            main { class: "main",
                div { class: "scroll", id: "library-scroll",
                    if !has_dir || total == 0 {
                        div { class: "empty",
                            div {
                                div { class: "big", "🗂️" }
                                h3 { if has_dir { "No ROMs found" } else { "Choose your ROM folder" } }
                                p { "Point mGBA at a folder of .gba files to build your library." }
                                button { class: "btn primary", onclick: move |_| pick_folder(state), "Choose folder…" }
                            }
                        }
                    }

                    if show_hero {
                        if let Some(h) = hero_game.clone() {
                            Hero { id: h.id, resume: hero_is_resume }
                        }
                        if !shelf.is_empty() {
                            div { class: "section-head", h2 { "Jump back in" } span { class: "sub", "Your recent adventures" } }
                            div { class: "shelf",
                                for (id, title, last, secs) in shelf {
                                    button {
                                        key: "{id}",
                                        class: "shelf-card",
                                        onclick: move |_| { state.launch(id, true); },
                                        div { class: "shot",
                                            img { src: "/art/resume/{id}?v={last}", alt: "", draggable: "false" }
                                            div { class: "play", "▶" }
                                        }
                                        div { class: "info",
                                            b { "{title}" }
                                            span { "{ago(last)} · {duration(secs)} played" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if total > 0 {
                        div { class: "section-head",
                            h2 { "{section_title}" }
                            span { class: "sub",
                                if shown == total { "{total} games" } else { "{shown} of {total}" }
                            }
                        }
                        if list.is_empty() {
                            div { class: "empty",
                                div {
                                    div { class: "big", "🫥" }
                                    h3 { "Nothing here yet" }
                                    p { "Try a different search or section." }
                                }
                            }
                        }
                        div { class: "grid", style: "--card: {card_size}px",
                            for (id, anchor, fav, saved) in cards {
                                Card { key: "{id}", id, anchor, fav, saved }
                            }
                        }
                    }
                }
                if alpha && shown > 40 {
                    div { class: "rail",
                        for l in "#ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
                            button {
                                key: "{l}",
                                disabled: !letters_present.contains(&l),
                                onclick: move |_| { document::eval(&format!("window.__lib.jump('{l}')")); },
                                "{l}"
                            }
                        }
                    }
                }
            }
        }

        if let Some(id) = selected {
            Details { id }
        }
        if settings_open() {
            Settings { on_close: move |_| settings_open.set(false) }
        }
    }
}

fn pick_folder(state: AppState) {
    spawn(async move {
        if let Some(dir) = rfd::AsyncFileDialog::new()
            .set_title("Choose your ROM folder")
            .pick_folder()
            .await
        {
            state.set_roms_dir(dir.path().to_path_buf());
        }
    });
}

#[component]
fn Hero(id: usize, resume: bool) -> Element {
    let state = use_context::<AppState>();
    let Some(g) = state.games.read().get(id).cloned() else { return rsx! {} };
    let stats = state.store.read().stats(&g);
    let shot = if resume {
        format!("/art/resume/{id}?v={}", stats.last_played)
    } else {
        format!("/art/title/{id}")
    };
    rsx! {
        div { class: "hero",
            div { class: "hero-bg", style: "background-image: url('/art/title/{id}')" }
            div { class: "hero-content",
                div { class: "hero-kicker", if resume { "Continue playing" } else { "Featured from your library" } }
                h1 { "{g.title}" }
                p {
                    if resume {
                        "Last played {ago(stats.last_played)} · {duration(stats.seconds)} total"
                    } else {
                        "{g.region_flags()} {g.platform} · {g.size / 1024 / 1024} MB"
                    }
                }
                div { class: "hero-actions",
                    button { class: "btn primary", onclick: move |_| { state.launch(id, resume); },
                        if resume { "▶  Continue" } else { "▶  Play" }
                    }
                    button { class: "btn", onclick: move |_| state.selected.clone().set(Some(id)), "Details" }
                }
            }
            div { class: "hero-shot",
                img { src: "{shot}", alt: "", draggable: "false" }
            }
        }
    }
}

#[component]
fn Card(id: usize, anchor: Option<char>, fav: bool, saved: bool) -> Element {
    let state = use_context::<AppState>();
    let mut loaded = use_signal(|| false);
    let mut failed = use_signal(|| false);
    let Some(g) = state.games.read().get(id).cloned() else { return rsx! {} };
    let h = hue(&g.title);
    let anchor_id = anchor.map(|c| format!("letter-{c}")).unwrap_or_default();
    let glyph = g.letter;
    let flags = g.region_flags();
    rsx! {
        div {
            class: "card",
            id: "{anchor_id}",
            tabindex: "0",
            onclick: move |_| state.selected.clone().set(Some(id)),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Enter { state.launch(id, false); }
            },
            div { class: "cover",
                div { class: "fallback", style: "--h: {h}",
                    div { class: "glyph", "{glyph}" }
                    div { class: "cart", "{g.platform}" }
                    div { class: "name", "{g.title}" }
                }
                if !failed() {
                    img {
                        class: if loaded() { "art loaded" } else { "art" },
                        src: "/art/boxart/{id}",
                        alt: "",
                        loading: "lazy",
                        draggable: "false",
                        onload: move |_| loaded.set(true),
                        onerror: move |_| failed.set(true),
                    }
                }
                div { class: "badges",
                    if fav { span { class: "badge fav", "★" } }
                    if saved { span { class: "badge", "💾" } }
                }
                button {
                    class: "play-fab",
                    title: "Play",
                    onclick: move |e| { e.stop_propagation(); state.launch(id, false); },
                    "▶"
                }
            }
            div { class: "meta",
                div { class: "title", title: "{g.stem}", "{g.title}" }
                div { class: "subline",
                    span { "{flags}" }
                    if let Some(rev) = &g.revision { span { class: "pill", "{rev}" } }
                    for t in g.tags.iter().take(2) { span { class: "pill warn", "{t}" } }
                }
            }
        }
    }
}

#[component]
fn Details(id: usize) -> Element {
    let state = use_context::<AppState>();
    let Some(g) = state.games.read().get(id).cloned() else { return rsx! {} };
    state.tick.read();
    let store = state.store.read();
    let stats = store.stats(&g);
    let fav = store.favorites.contains(&g.stem);
    drop(store);
    let can_resume = library::resume_path(&g).exists();
    let slots = g.state_slots();
    let has_save = g.has_save();
    let path = g.path.clone();
    let size_mb = g.size as f64 / 1024.0 / 1024.0;
    let close = move |_| state.selected.clone().set(None);

    rsx! {
        div { class: "backdrop", onclick: close }
        aside { class: "drawer",
            div { class: "drawer-hero",
                div { class: "bg", style: "background-image: url('/art/title/{id}')" }
                button { class: "drawer-close", onclick: close, "✕" }
                div { class: "box",
                    img { src: "/art/boxart/{id}", alt: "", draggable: "false" }
                }
            }
            div { class: "drawer-body",
                h2 { "{g.title}" }
                div { class: "stem", "{g.stem}" }
                div { style: "display:flex; gap:6px; flex-wrap:wrap; align-items:center",
                    span { style: "font-size:18px", "{g.region_flags()}" }
                    span { class: "pill", "{g.platform}" }
                    if let Some(rev) = &g.revision { span { class: "pill", "{rev}" } }
                    for t in g.tags.iter() { span { class: "pill warn", "{t}" } }
                    if g.hashes.as_ref().is_some_and(|h| h.verified) { span { class: "pill ok", "✓ No-Intro verified" } }
                    if has_save { span { class: "pill ok", "💾 Battery save" } }
                }
                div { class: "drawer-actions",
                    if can_resume {
                        button { class: "btn primary", onclick: move |_| { state.launch(id, true); }, "▶  Continue" }
                        button { class: "btn", onclick: move |_| { state.launch(id, false); }, "Start fresh" }
                    } else {
                        button { class: "btn primary", onclick: move |_| { state.launch(id, false); }, "▶  Play" }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| state.toggle_favorite(id),
                        if fav { "★  Favorited" } else { "☆  Favorite" }
                    }
                    button { class: "btn", onclick: move |_| reveal_in_finder(&path), "Show file" }
                }

                div { class: "shots",
                    div { class: "frame", img { src: "/art/title/{id}", alt: "", draggable: "false" } span { "Title" } }
                    div { class: "frame", img { src: "/art/snap/{id}", alt: "", draggable: "false" } span { "In-game" } }
                }

                div { class: "stat-row",
                    div { class: "stat", b { "{stats.plays}" } span { "sessions" } }
                    div { class: "stat", b { "{duration(stats.seconds)}" } span { "played" } }
                    div { class: "stat", b { "{ago(stats.last_played)}" } span { "last played" } }
                }

                if !slots.is_empty() {
                    div { class: "h3", "Save states" }
                    div { class: "slots",
                        for (slot, mtime) in slots {
                            button {
                                key: "{slot}",
                                class: "slot",
                                title: "Play from slot {slot}",
                                onclick: move |_| {
                                    if state.launch(id, false) {
                                        if let Some(s) = state.session.peek().as_ref() {
                                            s.send(crate::emu::Cmd::LoadState(slot));
                                        }
                                    }
                                },
                                div { class: "thumb", img { src: "/art/state/{id}/{slot}?v={mtime}", alt: "", draggable: "false" } }
                                div { class: "cap", b { "Slot {slot}" } span { "{ago(mtime)}" } }
                            }
                        }
                    }
                }

                div { class: "h3", "Details" }
                dl { class: "facts",
                    dt { "Regions" } dd { if g.regions.is_empty() { "—" } else { "{g.regions.join(\", \")}" } }
                    if let Some(l) = &g.languages { dt { "Languages" } dd { "{l}" } }
                    dt { "File size" } dd { "{size_mb:.1} MB" }
                    dt { "Internal title" } dd { class: "mono", "{g.header.internal_title}" }
                    dt { "Game code" } dd { class: "mono", "{g.header.game_code}" }
                    dt { "Maker / version" } dd { class: "mono", "{g.header.maker} · v{g.header.version}" }
                    if let Some(h) = &g.hashes {
                        dt { "CRC32" } dd { class: "mono", "{h.crc}" }
                        dt { "MD5" } dd { class: "mono", "{h.md5}" }
                        dt { "SHA-1" } dd { class: "mono", "{h.sha1}" }
                    }
                }
            }
        }
    }
}

#[component]
fn Settings(on_close: EventHandler<()>) -> Element {
    let state = use_context::<AppState>();
    let settings = state.store.read().settings.clone();
    let dir = state
        .roms_dir
        .read()
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "Not set".into());
    let filter = settings.video_filter.clone();
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
        div { class: "backdrop", onclick: move |_| on_close.call(()) }
        div { class: "modal",
            h2 { "Settings" }
            div { class: "field",
                div { class: "lbl", "ROM folder" small { "{dir}" } }
                button { class: "btn small", onclick: move |_| pick_folder(state), "Change…" }
                button {
                    class: "btn small",
                    onclick: move |_| {
                        if let Some(d) = state.roms_dir.peek().clone() { state.set_roms_dir(d) }
                    },
                    "Rescan"
                }
            }
            div { class: "field",
                div { class: "lbl", "Video filter" small { "How the GBA screen is drawn" } }
                div { class: "seg",
                    {seg("pixel", "Pixel")}
                    {seg("smooth", "Smooth")}
                    {seg("lcd", "LCD")}
                    {seg("crt", "CRT")}
                }
            }
            div { class: "field",
                div { class: "lbl", "Integer scaling" small { "Crisp, evenly sized pixels" } }
                button {
                    class: if settings.integer_scale { "switch on" } else { "switch" },
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
                div { class: "lbl", "Volume" small { "{settings.volume}%" } }
                input {
                    class: "range",
                    r#type: "range", min: "0", max: "100",
                    value: "{settings.volume}",
                    oninput: move |e| {
                        let v = e.value().parse().unwrap_or(80);
                        state.update_settings(|s| s.volume = v);
                    },
                }
            }
            div { class: "field",
                div { class: "lbl", "Artwork cache" small { "Box art from libretro-thumbnails" } }
                button {
                    class: "btn small danger",
                    onclick: move |_| { art::clear_cache(); state.toast("Artwork cache cleared — restart to refetch"); },
                    "Clear"
                }
            }
            div { class: "h3", "Controls" }
            ControlsTable {}
            div { style: "display:flex; justify-content:flex-end; margin-top: 20px",
                button { class: "btn primary", onclick: move |_| on_close.call(()), "Done" }
            }
        }
    }
}

#[component]
pub fn ControlsTable() -> Element {
    let rows = [
        ("D-Pad", "Arrow keys"),
        ("A", "X"),
        ("B", "Z"),
        ("L / R", "A / S"),
        ("Start", "Enter"),
        ("Select", "Backspace"),
        ("Fast-forward (hold)", "Tab"),
        ("Pause menu", "Esc"),
        ("Load state 1–9", "F1–F9"),
        ("Save state 1–9", "⇧ F1–F9"),
        ("Screenshot", "F12"),
        ("Gamepad menu", "Home / Select+Start"),
    ];
    rsx! {
        div { class: "keys",
            for (what, key) in rows {
                div { key: "{what}", span { "{what}" } kbd { "{key}" } }
            }
        }
    }
}

