//! ROM library: scanning a folder of No-Intro named ROMs, parsing metadata,
//! and persisting favourites / play statistics / settings.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const ROM_EXTENSIONS: &[&str] = &["gba", "gb", "gbc"];
pub const STATE_SLOTS: u8 = 9;

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub id: usize,
    /// File stem, e.g. `Legend of Zelda, The - The Minish Cap (USA)`; the No-Intro name.
    pub stem: String,
    pub path: PathBuf,
    /// Human friendly title, e.g. `The Legend of Zelda - The Minish Cap`.
    pub title: String,
    pub sort_key: String,
    /// First letter used by the A–Z rail (`#` for digits/symbols).
    pub letter: char,
    /// Franchise / series guess: text before the first ` - `.
    pub series: String,
    pub regions: Vec<String>,
    pub languages: Option<String>,
    pub revision: Option<String>,
    /// Remaining parenthesised tags: `Tr`, `Beta`, `Proto`, `Unl`, ...
    pub tags: Vec<String>,
    pub size: u64,
    pub header: RomHeader,
    pub hashes: Option<Hashes>,
    pub platform: &'static str,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RomHeader {
    pub internal_title: String,
    pub game_code: String,
    pub maker: String,
    pub version: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hashes {
    pub crc: String,
    pub md5: String,
    pub sha1: String,
    pub verified: bool,
}

impl Game {
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    pub fn save_path(&self) -> PathBuf {
        self.dir().join(format!("{}.sav", self.stem))
    }

    /// mGBA-compatible slot naming: `<rom stem>.ss<N>` next to the ROM.
    pub fn state_path(&self, slot: u8) -> PathBuf {
        self.dir().join(format!("{}.ss{}", self.stem, slot))
    }

    pub fn has_save(&self) -> bool {
        std::fs::metadata(self.save_path())
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    }

    /// Existing state slots with their modification time (unix secs).
    pub fn state_slots(&self) -> Vec<(u8, u64)> {
        (1..=STATE_SLOTS)
            .filter_map(|slot| {
                let m = std::fs::metadata(self.state_path(slot)).ok()?;
                Some((slot, mtime_secs(&m)))
            })
            .collect()
    }

    pub fn region_flags(&self) -> String {
        self.regions.iter().filter_map(|r| region_flag(r)).collect()
    }

    pub fn is_translation(&self) -> bool {
        self.tags.iter().any(|t| {
            matches!(t.as_str(), "Tr" | "T" | "Hack") || t.starts_with("T+") || t.starts_with("T-")
        })
    }
}

pub fn mtime_secs(m: &std::fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn region_flag(region: &str) -> Option<&'static str> {
    Some(match region {
        "USA" => "🇺🇸",
        "Europe" => "🇪🇺",
        "Japan" => "🇯🇵",
        "World" => "🌐",
        "Germany" => "🇩🇪",
        "France" => "🇫🇷",
        "Spain" => "🇪🇸",
        "Italy" => "🇮🇹",
        "Australia" => "🇦🇺",
        "Korea" => "🇰🇷",
        "China" => "🇨🇳",
        "Canada" => "🇨🇦",
        "Netherlands" => "🇳🇱",
        "Sweden" => "🇸🇪",
        "Brazil" => "🇧🇷",
        "UK" => "🇬🇧",
        _ => return None,
    })
}

const KNOWN_REGIONS: &[&str] = &[
    "USA", "Europe", "Japan", "World", "Germany", "France", "Spain", "Italy", "Australia", "Korea",
    "China", "Canada", "Netherlands", "Sweden", "Brazil", "UK", "Asia", "Scandinavia",
];

const LANGUAGES: &[&str] = &[
    "En", "Fr", "De", "Es", "It", "Ja", "Nl", "Sv", "Pt", "Da", "No", "Fi", "Zh", "Ko", "Pl", "Ru",
    "Ca", "El", "Hu", "Cs",
];

/// Parse a No-Intro style name into (title, regions, languages, revision, tags).
pub fn parse_name(stem: &str) -> (String, Vec<String>, Option<String>, Option<String>, Vec<String>) {
    let (base, rest) = match stem.find(" (") {
        Some(i) => (&stem[..i], &stem[i..]),
        None => (stem, ""),
    };
    let mut regions = Vec::new();
    let mut languages = None;
    let mut revision = None;
    let mut tags = Vec::new();
    for group in rest.split('(').skip(1) {
        let Some(inner) = group.split(')').next() else { continue };
        let inner = inner.trim();
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if regions.is_empty() && parts.iter().all(|p| KNOWN_REGIONS.contains(p)) {
            regions = parts.iter().map(|s| s.to_string()).collect();
        } else if parts.iter().all(|p| LANGUAGES.contains(p)) {
            languages = Some(parts.join(", "));
        } else if let Some(r) = inner.strip_prefix("Rev ") {
            revision = Some(format!("Rev {r}"));
        } else if !inner.is_empty() {
            tags.push(inner.to_string());
        }
    }
    (pretty_title(base), regions, languages, revision, tags)
}

/// `Legend of Zelda, The - The Minish Cap` -> `The Legend of Zelda - The Minish Cap`.
fn pretty_title(base: &str) -> String {
    let mut segments: Vec<String> = base
        .split(" - ")
        .map(|seg| {
            for article in [", The", ", A", ", An"] {
                if let Some(head) = seg.strip_suffix(article) {
                    return format!("{} {}", &article[2..], head);
                }
            }
            seg.to_string()
        })
        .collect();
    if segments.is_empty() {
        segments.push(base.to_string());
    }
    segments.join(" - ")
}

fn sort_key(title: &str) -> String {
    let lower = title.to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            return rest.to_string();
        }
    }
    lower
}

fn read_header(path: &Path) -> RomHeader {
    let mut buf = [0u8; 0xC0];
    let ok = std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if !ok {
        return RomHeader::default();
    }
    let text = |r: std::ops::Range<usize>| {
        String::from_utf8_lossy(&buf[r])
            .trim_matches(|c: char| c == '\0' || c.is_whitespace())
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
    };
    RomHeader {
        internal_title: text(0xA0..0xAC),
        game_code: text(0xAC..0xB0),
        maker: text(0xB0..0xB2),
        version: buf[0xBC],
    }
}

/// Vimm's Lair style info files (`<stem>.txt`) carry No-Intro verified hashes.
fn read_hashes(path: &Path) -> Option<Hashes> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut h = Hashes::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("CRC:") {
            h.crc = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("MD5:") {
            h.md5 = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("SHA-1:") {
            h.sha1 = v.trim().to_string();
        } else if line.contains("verified by No-Intro") {
            h.verified = true;
        }
    }
    (!h.crc.is_empty() || !h.md5.is_empty()).then_some(h)
}

pub fn scan(dir: &Path) -> Vec<Game> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut games: Vec<Game> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            if !ROM_EXTENSIONS.contains(&ext.as_str()) {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_string();
            let (title, regions, languages, revision, tags) = parse_name(&stem);
            let sort = sort_key(&title);
            let letter = sort
                .chars()
                .next()
                .filter(|c| c.is_ascii_alphabetic())
                .map_or('#', |c| c.to_ascii_uppercase());
            let series = title.split(" - ").next().unwrap_or(&title).trim().to_string();
            Some(Game {
                id: 0,
                size: e.metadata().map(|m| m.len()).unwrap_or(0),
                header: read_header(&path),
                hashes: read_hashes(&dir.join(format!("{stem}.txt"))),
                platform: match ext.as_str() {
                    "gba" => "GBA",
                    "gbc" => "GBC",
                    _ => "GB",
                },
                stem,
                path,
                title,
                sort_key: sort,
                letter,
                series,
                regions,
                languages,
                revision,
                tags,
            })
        })
        .collect();
    games.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.stem.cmp(&b.stem)));
    for (i, g) in games.iter_mut().enumerate() {
        g.id = i;
    }
    games
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Stats {
    pub plays: u32,
    pub last_played: u64,
    pub seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub roms_dir: Option<PathBuf>,
    pub volume: u32,
    pub video_filter: String,
    pub integer_scale: bool,
    pub card_size: u32,
    pub show_fps: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            roms_dir: None,
            volume: 80,
            video_filter: "pixel".into(),
            integer_scale: false,
            card_size: 170,
            show_fps: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Store {
    pub favorites: BTreeSet<String>,
    pub stats: BTreeMap<String, Stats>,
    pub settings: Settings,
}

pub fn data_dir() -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mgba-dioxus");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Where we keep the automatic "continue where you left off" states.
pub fn resume_path(game: &Game) -> PathBuf {
    let dir = data_dir().join("resume");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{}.ss0", game.stem))
}

pub fn screenshot_dir() -> PathBuf {
    let dir = data_dir().join("screenshots");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

impl Store {
    fn path() -> PathBuf {
        data_dir().join("library.json")
    }

    pub fn load() -> Store {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let tmp = Self::path().with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(tmp, Self::path());
            }
        }
    }

    pub fn stats(&self, game: &Game) -> Stats {
        self.stats.get(&game.stem).cloned().unwrap_or_default()
    }
}

/// Pick the ROM directory: CLI arg > $MGBA_ROMS > saved setting > common defaults.
pub fn resolve_roms_dir(store: &Store) -> Option<PathBuf> {
    let from_args = std::env::args().skip(1).find(|a| !a.starts_with('-')).map(PathBuf::from);
    let from_env = std::env::var_os("MGBA_ROMS").map(PathBuf::from);
    let candidates = [
        from_args,
        from_env,
        store.settings.roms_dir.clone(),
        dirs::home_dir().map(|h| h.join("dev/gba/roms")),
        dirs::home_dir().map(|h| h.join("roms")),
        Some(PathBuf::from("roms")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_no_intro_names() {
        let (t, r, l, rev, tags) =
            parse_name("Legend of Zelda, The - The Minish Cap (USA) (En,Fr) (Rev 1) (Beta)");
        assert_eq!(t, "The Legend of Zelda - The Minish Cap");
        assert_eq!(r, vec!["USA"]);
        assert_eq!(l.as_deref(), Some("En, Fr"));
        assert_eq!(rev.as_deref(), Some("Rev 1"));
        assert_eq!(tags, vec!["Beta"]);

        let (t, r, ..) = parse_name("Pokemon - LeafGreen Version (USA, Europe) (Rev 1)");
        assert_eq!(t, "Pokemon - LeafGreen Version");
        assert_eq!(r, vec!["USA", "Europe"]);

        let (t, _, _, _, tags) = parse_name("Mother 3 (Tr)");
        assert_eq!(t, "Mother 3");
        assert_eq!(tags, vec!["Tr"]);

        assert_eq!(sort_key("The Legend of Zelda"), "legend of zelda");
    }
}
