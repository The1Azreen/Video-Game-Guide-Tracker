//! Games, their guides and where you are in each one, plus loading/saving `library.json`.

use std::path::{Path, PathBuf};

use egui::Color32;
use serde::{Deserialize, Serialize};

use crate::links;
use crate::theme::*;

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Playing,
    Backlog,
    Paused,
    Completed,
    Dropped,
}

impl Status {
    pub const ALL: [Status; 5] = [Status::Playing, Status::Backlog, Status::Paused, Status::Completed, Status::Dropped];

    pub fn label(self) -> &'static str {
        match self {
            Status::Playing => "Playing",
            Status::Backlog => "Backlog",
            Status::Paused => "Paused",
            Status::Completed => "Completed",
            Status::Dropped => "Dropped",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Status::Playing => MAGENTA,
            Status::Backlog => TEAL,
            Status::Paused => YELLOW,
            Status::Completed => Color32::from_rgb(0x8C, 0xF0, 0x8C),
            Status::Dropped => DIM,
        }
    }

    /// Sort order in the games list: what you're playing comes first.
    pub fn rank(self) -> u8 {
        match self {
            Status::Playing => 0,
            Status::Paused => 1,
            Status::Backlog => 2,
            Status::Completed => 3,
            Status::Dropped => 4,
        }
    }
}

/// What sort of guide a link is. Decides how "resume" works.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Web,
    Video,
    Sheet,
    Wiki,
    Trophy,
    Map,
    Doc,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Web, Kind::Video, Kind::Sheet, Kind::Wiki, Kind::Trophy, Kind::Map, Kind::Doc];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Web => "Web guide",
            Kind::Video => "Video",
            Kind::Sheet => "Spreadsheet",
            Kind::Wiki => "Wiki",
            Kind::Trophy => "Trophy guide",
            Kind::Map => "Map",
            Kind::Doc => "Document",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum StepKind {
    #[default]
    Task,
    Chest,
    Objective,
    Achievement,
}

impl StepKind {
    pub const ALL: [StepKind; 4] = [StepKind::Chest, StepKind::Objective, StepKind::Achievement, StepKind::Task];

    pub fn plural(self) -> &'static str {
        match self {
            StepKind::Task => "Other",
            StepKind::Chest => "Chests",
            StepKind::Objective => "Objectives",
            StepKind::Achievement => "Achievements",
        }
    }
}

/// One checklist item. Only `text` is required; the rest comes from pasting a structured
/// guide (area headers, "How:", "Reward:" lines...).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    /// The item's name, e.g. "Objective 14: Frozen Rail Ride".
    pub text: String,
    pub done: bool,
    pub kind: StepKind,
    /// Area / chapter heading the item sits under.
    pub section: String,
    /// What to do, e.g. the objective's requirement or the chest's location.
    pub detail: String,
    pub how: String,
    pub reward: String,
    pub hidden: bool,
    /// Text on the guide's page to scroll to for this item.
    pub anchor: String,
}

impl Step {
    pub fn task(text: impl Into<String>) -> Self {
        Step { text: text.into(), ..Default::default() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Guide {
    pub id: u64,
    pub title: String,
    pub url: String,
    pub kind: Kind,
    /// Free text: "Chapter 4, after the bridge", "Mission 12"...
    pub position: String,
    /// Videos: how far in you are, in seconds.
    pub video_secs: u32,
    /// Spreadsheets: the row you're on.
    pub sheet_row: u32,
    /// Web pages: a phrase on the page to scroll to when reopening it.
    pub jump_text: String,
    /// Manual progress, used when the guide has no checklist.
    pub percent: f32,
    pub steps: Vec<Step>,
    pub notes: String,
    pub done: bool,
    pub added: i64,
    pub last_opened: Option<i64>,
    /// True until a real title has been fetched or typed.
    pub auto_title: bool,
}

impl Guide {
    pub fn new(id: u64, url: String) -> Self {
        let kind = links::detect_kind(&url);
        let mut g = Guide {
            id,
            title: links::fallback_title(&url),
            kind,
            added: now(),
            auto_title: true,
            ..Default::default()
        };
        // Keep the place the link already points at.
        if let Some(t) = links::youtube_time(&url) {
            g.video_secs = t;
        }
        if let Some(r) = links::sheet_row(&url) {
            g.sheet_row = r;
        }
        g.url = url;
        g
    }

    pub fn progress(&self) -> f32 {
        if self.done {
            1.0
        } else if !self.steps.is_empty() {
            self.steps.iter().filter(|s| s.done).count() as f32 / self.steps.len() as f32
        } else {
            self.percent.clamp(0.0, 1.0)
        }
    }

    /// Short description of the saved spot, e.g. "at 1:02:03" or "row 132".
    pub fn spot(&self) -> String {
        let mut parts = Vec::new();
        match self.kind {
            Kind::Video if self.video_secs > 0 => parts.push(format!("at {}", links::fmt_hms(self.video_secs))),
            Kind::Sheet if self.sheet_row > 0 => parts.push(format!("row {}", self.sheet_row)),
            _ if !self.jump_text.trim().is_empty() => parts.push(format!("\"{}\"", self.jump_text.trim())),
            _ => {}
        }
        if !self.position.trim().is_empty() {
            parts.push(self.position.trim().to_string());
        }
        if let Some(n) = self.next_step().filter(|_| parts.is_empty()) {
            parts.push(format!("next: {}", n.text));
        }
        parts.join(" · ")
    }

    pub fn next_step(&self) -> Option<&Step> {
        self.steps.iter().find(|s| !s.done)
    }

    /// After ticking things off, point Resume at the next unticked item.
    pub fn follow_checklist(&mut self) {
        if self.kind == Kind::Video || self.kind == Kind::Sheet {
            return;
        }
        let (anchor, section) = match self.next_step() {
            Some(n) => (n.anchor.clone(), n.section.clone()),
            None => return,
        };
        if !anchor.is_empty() {
            self.jump_text = anchor;
        }
        if !section.is_empty() {
            self.position = section;
        }
    }

    pub fn resume_url(&self) -> String {
        links::resume_url(self)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogEntry {
    pub at: i64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    pub id: u64,
    pub title: String,
    pub platform: String,
    pub status: Status,
    /// "Where I left off": the note you read first when you come back.
    pub resume_note: String,
    pub log: Vec<LogEntry>,
    pub guides: Vec<Guide>,
    pub added: i64,
    pub last_played: Option<i64>,
    /// The guide "Continue" reopens.
    pub last_guide: Option<u64>,
}

impl Game {
    pub fn progress(&self) -> f32 {
        if self.status == Status::Completed {
            return 1.0;
        }
        if self.guides.is_empty() {
            return 0.0;
        }
        self.guides.iter().map(Guide::progress).sum::<f32>() / self.guides.len() as f32
    }

    pub fn continue_guide(&self) -> Option<&Guide> {
        self.last_guide
            .and_then(|id| self.guides.iter().find(|g| g.id == id))
            .or_else(|| self.guides.iter().find(|g| !g.done))
            .or_else(|| self.guides.first())
    }

    pub fn touch(&mut self) {
        self.last_played = Some(now());
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub version: u32,
    pub next_id: u64,
    pub games: Vec<Game>,
}

impl Default for Library {
    fn default() -> Self {
        Library { version: 1, next_id: 1, games: Vec::new() }
    }
}

impl Library {
    pub fn new_id(&mut self) -> u64 {
        // Guard against hand-edited files whose ids are ahead of the counter.
        let max = self.games.iter().flat_map(|g| std::iter::once(g.id).chain(g.guides.iter().map(|x| x.id))).max().unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn game_mut(&mut self, id: u64) -> Option<&mut Game> {
        self.games.iter_mut().find(|g| g.id == id)
    }

    /// Adds every game from `other` with fresh ids. Returns how many were added.
    pub fn merge(&mut self, other: Library) -> usize {
        let n = other.games.len();
        for mut g in other.games {
            g.id = self.new_id();
            let old_last = g.last_guide;
            g.last_guide = None;
            for guide in &mut g.guides {
                let old = guide.id;
                guide.id = self.new_id();
                if old_last == Some(old) {
                    g.last_guide = Some(guide.id);
                }
            }
            self.games.push(g);
        }
        n
    }
}

// ---------------------------------------------------------------------------------------------
// Storage

/// Portable mode: if a `jagt-data` folder sits next to the executable (or the executable's name
/// contains "portable"), everything lives there instead of the user profile.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = local_dir() {
        return dir;
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("JAGT");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Where data lives when it isn't in the user profile: portable mode, or a build run from this
/// repo (`cargo run`), which keeps it in `<repo>/jagt-data` (gitignored).
pub fn local_dir() -> Option<PathBuf> {
    portable_dir().or_else(repo_dir)
}

fn repo_dir() -> Option<PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let repo = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).ok()?;
    if !exe.starts_with(repo.join("target")) {
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("jagt-data");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("jagt-data");
    let named_portable = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_ascii_lowercase().contains("portable"));
    if dir.is_dir() || (named_portable && std::fs::create_dir_all(&dir).is_ok()) {
        Some(dir)
    } else {
        None
    }
}

pub fn library_path() -> PathBuf {
    data_dir().join("library.json")
}

/// Loads the library. A file that can't be parsed is moved aside (never overwritten) and the
/// returned message says where it went.
pub fn load(path: &Path) -> (Library, Option<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Library::default(), None);
    };
    match serde_json::from_str::<Library>(&text) {
        Ok(lib) => (lib, None),
        Err(e) => {
            let aside = path.with_file_name(format!("library.unreadable-{}.json", now()));
            let _ = std::fs::rename(path, &aside);
            (Library::default(), Some(format!("library.json couldn't be read ({e}); kept it as {}", aside.display())))
        }
    }
}

pub fn read_file(path: &Path) -> Result<Library, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Writes atomically, keeping the previous version as `library.bak.json`.
pub fn save(path: &Path, lib: &Library) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(lib).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        let _ = std::fs::copy(path, path.with_file_name("library.bak.json"));
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

pub fn ago(ts: i64) -> String {
    let d = now() - ts;
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{} min ago", d / 60)
    } else if d < 86_400 {
        format!("{} h ago", d / 3600)
    } else if d < 2 * 86_400 {
        "yesterday".into()
    } else if d < 30 * 86_400 {
        format!("{} days ago", d / 86_400)
    } else {
        date(ts)
    }
}

pub fn date(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(t) => t.format("%d %b %Y").to_string(),
        None => "?".into(),
    }
}

pub fn date_time(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(t) => t.format("%a %d %b %Y, %H:%M").to_string(),
        None => "?".into(),
    }
}
