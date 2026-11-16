//! Box art / title screen / snapshot images from the libretro-thumbnails
//! project, fetched on demand by a small worker pool and cached on disk.
//!
//! The webview requests `/art/<kind>/<game id>` and the asset handler hands the
//! request (and its async responder) to this service.

use crate::library::data_dir;
use dioxus::desktop::wry::{http::Response, RequestAsyncResponder};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

const BASE: &str = "https://raw.githubusercontent.com/libretro-thumbnails";
const WORKERS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtKind {
    Boxart,
    Title,
    Snap,
}

impl ArtKind {
    pub fn parse(s: &str) -> Option<ArtKind> {
        Some(match s {
            "boxart" => ArtKind::Boxart,
            "title" => ArtKind::Title,
            "snap" => ArtKind::Snap,
            _ => return None,
        })
    }

    fn folder(self) -> &'static str {
        match self {
            ArtKind::Boxart => "Named_Boxarts",
            ArtKind::Title => "Named_Titles",
            ArtKind::Snap => "Named_Snaps",
        }
    }
}

fn system_repo(platform: &str) -> &'static str {
    match platform {
        "GB" => "Nintendo_-_Game_Boy",
        "GBC" => "Nintendo_-_Game_Boy_Color",
        _ => "Nintendo_-_Game_Boy_Advance",
    }
}

/// libretro replaces these characters with `_` in thumbnail file names.
fn libretro_name(name: &str) -> String {
    name.chars()
        .map(|c| if "&*/:`<>?\\|\"".contains(c) { '_' } else { c })
        .collect()
}

/// Names to try, most specific first: the full No-Intro name, then with
/// trailing tags (revision, Beta, ...) progressively removed.
fn candidates(stem: &str, is_translation: bool) -> Vec<String> {
    let mut out = vec![stem.to_string()];
    let mut cur = stem.to_string();
    while let Some(i) = cur.rfind(" (") {
        let shorter = cur[..i].to_string();
        // Never strip the last group (normally the region) unless it's a translation tag.
        if !shorter.contains(" (") {
            if is_translation {
                out.push(format!("{shorter} (Japan)"));
            }
            break;
        }
        out.push(shorter.clone());
        cur = shorter;
    }
    out.dedup();
    out
}

struct Job {
    kind: ArtKind,
    stem: String,
    platform: &'static str,
    translation: bool,
    responder: RequestAsyncResponder,
}

#[derive(Clone)]
pub struct ArtService {
    tx: mpsc::Sender<Job>,
}

impl ArtService {
    pub fn new() -> ArtService {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..WORKERS {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("art-{i}"))
                .spawn(move || loop {
                    let job = match rx.lock().unwrap().recv() {
                        Ok(job) => job,
                        Err(_) => return,
                    };
                    let bytes = fetch(job.kind, &job.stem, job.platform, job.translation);
                    respond_image(job.responder, bytes, "image/png", true);
                })
                .expect("spawn art worker");
        }
        ArtService { tx }
    }

    pub fn request(
        &self,
        kind: ArtKind,
        stem: &str,
        platform: &'static str,
        translation: bool,
        responder: RequestAsyncResponder,
    ) {
        // Fast path: cache hit/miss answered without a thread hop.
        let path = cache_path(kind, stem);
        if let Ok(bytes) = std::fs::read(&path) {
            return respond_image(responder, Some(bytes), "image/png", true);
        }
        if path.with_extension("miss").exists() {
            return respond_image(responder, None, "image/png", true);
        }
        let _ = self.tx.send(Job {
            kind,
            stem: stem.to_string(),
            platform,
            translation,
            responder,
        });
    }
}

fn cache_path(kind: ArtKind, stem: &str) -> PathBuf {
    let dir = data_dir().join("art").join(kind.folder());
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{}.png", libretro_name(stem)))
}

pub fn clear_cache() {
    let _ = std::fs::remove_dir_all(data_dir().join("art"));
}

fn fetch(kind: ArtKind, stem: &str, platform: &'static str, translation: bool) -> Option<Vec<u8>> {
    let path = cache_path(kind, stem);
    if let Ok(bytes) = std::fs::read(&path) {
        return Some(bytes);
    }
    let mut network_error = false;
    for name in candidates(stem, translation) {
        let url = format!(
            "{BASE}/{}/master/{}/{}.png",
            system_repo(platform),
            kind.folder(),
            percent_encoding::utf8_percent_encode(
                &libretro_name(&name),
                percent_encoding::NON_ALPHANUMERIC
            )
        );
        match ureq::get(&url).call() {
            Ok(mut resp) => {
                if let Ok(bytes) = resp.body_mut().read_to_vec() {
                    if bytes.starts_with(b"\x89PNG") {
                        let _ = std::fs::write(&path, &bytes);
                        return Some(bytes);
                    }
                }
            }
            Err(ureq::Error::StatusCode(404)) => {}
            Err(_) => network_error = true,
        }
    }
    // Remember genuine misses so we don't hammer GitHub; retry later on network errors.
    if !network_error {
        let _ = std::fs::write(path.with_extension("miss"), b"");
    }
    None
}

pub fn respond_image(
    responder: RequestAsyncResponder,
    bytes: Option<Vec<u8>>,
    mime: &str,
    cacheable: bool,
) {
    let cache = if cacheable { "max-age=86400" } else { "no-store" };
    let response = match bytes {
        Some(bytes) => Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .header("Cache-Control", cache)
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes),
        None => Response::builder()
            .status(404)
            .header("Cache-Control", cache)
            .body(Vec::new()),
    };
    responder.respond(response.unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_names() {
        assert_eq!(
            candidates("Pokemon - LeafGreen Version (USA, Europe) (Rev 1)", false),
            vec![
                "Pokemon - LeafGreen Version (USA, Europe) (Rev 1)",
                "Pokemon - LeafGreen Version (USA, Europe)"
            ]
        );
        assert_eq!(candidates("Mother 3 (Tr)", true), vec!["Mother 3 (Tr)", "Mother 3 (Japan)"]);
        assert_eq!(libretro_name("Game & Watch: X?"), "Game _ Watch_ X_");
    }
}
