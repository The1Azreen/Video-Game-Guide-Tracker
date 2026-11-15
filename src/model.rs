//! Games, their guides and where you are in each one, plus loading/saving `library.json`.

use std::path::{Path, PathBuf};

use egui::Color32;
use serde::{Deserialize, Serialize};

use crate::links;
use crate::theme::*;

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The explorer's top-level folders.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Folder {
    Pc,
    Console,
    Mobile,
    Linux,
    Other,
}

impl Folder {
    pub const ALL: [Folder; 5] = [Folder::Pc, Folder::Console, Folder::Mobile, Folder::Linux, Folder::Other];

    pub fn label(self) -> &'static str {
        match self {
            Folder::Pc => "PC",
            Folder::Console => "Console",
            Folder::Mobile => "Mobile",
            Folder::Linux => "Linux",
            Folder::Other => "Other",
        }
    }

    /// Which folder a platform name belongs in: "PS5" → Console, "PC (Steam)" → PC,
    /// "Switch 2" / "PSP" → Console, "Android" → Mobile, "Steam Deck" → Linux.
    pub fn of_platform(platform: &str) -> Folder {
        let p = platform.to_lowercase();
        let words: Vec<&str> = p.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        let has = |w: &str| words.contains(&w);
        let any = |list: &[&str]| list.iter().any(|x| p.contains(x));
        if p.trim().is_empty() {
            Folder::Other
        } else if any(&["linux", "steam deck", "steamdeck", "steamos", "ubuntu", "proton"]) {
            Folder::Linux
        } else if any(&["android", "iphone", "ipad", "ios", "mobile", "phone", "apple arcade"]) {
            Folder::Mobile
        } else if any(&["rog ally", "legion go", "msi claw"]) {
            Folder::Pc
        } else if any(&["playstation", "xbox", "nintendo", "wii", "gamecube", "n64", "snes", "nes", "sega", "genesis", "mega drive", "dreamcast", "saturn", "atari", "turbografx"])
            || any(&["switch", "psp", "vita", "3ds", "game boy", "gameboy", "gba", "nds", "game gear", "neo geo pocket", "wonderswan"])
            || has("ds")
            || has("gb")
            || has("gbc")
            || words.iter().any(|w| w.starts_with("ps") && w[2..].chars().all(|c| c.is_ascii_digit()))
            || has("ps")
            || has("gc")
        {
            Folder::Console
        } else if any(&["pc", "windows", "steam", "epic", "gog", "browser", "flash", "mac", "itch", "battle.net", "origin", "ea app", "ubisoft"]) {
            Folder::Pc
        } else {
            Folder::Other
        }
    }
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
    /// Side content (quests, paralogues...). "Next" follows the non-optional items.
    pub optional: bool,
    /// Last in-game date it can be done, as "M/D" (e.g. "9/25"). Empty = no deadline.
    pub deadline: String,
    /// Marked as missed / skipped by hand.
    pub missed: bool,
    /// The page this item is on, when a guide spans several pages (one per chapter / week).
    /// Empty = the guide's own link.
    pub url: String,
    /// Your own note on this item ("came back for this after chapter 5").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// A screenshot or map attached to this item (an image file on this PC).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub image: String,
    /// Achievements: the share of players who have it (from Steam / Xbox), in percent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rarity: Option<f32>,
}

/// "9/25" → (9, 25). In-game calendars (Fire Emblem, Persona...) are month/day.
pub fn parse_md(s: &str) -> Option<(u32, u32)> {
    let (m, d) = s.trim().split_once('/')?;
    let (m, d) = (m.trim().parse().ok()?, d.trim().parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((m, d))
}

/// Whole days from `a` to `b` on a 31-day-month calendar (close enough for "due soon").
fn days_between(a: (u32, u32), b: (u32, u32)) -> i64 {
    (b.0 as i64 * 31 + b.1 as i64) - (a.0 as i64 * 31 + a.1 as i64)
}

/// Where an item stands, given the game's current in-game date.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepState {
    Open,
    /// Deadline within two weeks of the in-game date.
    DueSoon,
    Done,
    /// Marked missed, or its deadline has passed.
    Missed,
}

impl Step {
    pub fn task(text: impl Into<String>) -> Self {
        Step { text: text.into(), ..Default::default() }
    }

    pub fn state(&self, today: Option<(u32, u32)>) -> StepState {
        if self.done {
            return StepState::Done;
        }
        if self.missed {
            return StepState::Missed;
        }
        match (today, parse_md(&self.deadline)) {
            (Some(t), Some(d)) if days_between(t, d) < 0 => StepState::Missed,
            (Some(t), Some(d)) if days_between(t, d) <= 14 => StepState::DueSoon,
            _ => StepState::Open,
        }
    }

    /// Still to do (not done, not missed).
    pub fn is_open(&self, today: Option<(u32, u32)>) -> bool {
        matches!(self.state(today), StepState::Open | StepState::DueSoon)
    }

    /// Can be missed: it has a deadline, or the guide calls it missable.
    pub fn is_missable(&self) -> bool {
        !self.deadline.is_empty() || [&self.text, &self.detail, &self.how].iter().any(|t| t.to_lowercase().contains("missable"))
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
    /// For multi-page guides: the page Resume opens (the one the next item is on).
    pub resume_page: String,
    /// The last link check found the page gone (404, or the site no longer exists).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dead: bool,
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
            // missed items no longer count against you
            let counted = self.steps.iter().filter(|s| !s.missed).count().max(1);
            self.steps.iter().filter(|s| s.done).count() as f32 / counted as f32
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

    /// "I'm up to here": tick every item above `i` (in list order).
    pub fn tick_before(&mut self, i: usize) {
        let end = i.min(self.steps.len());
        for s in &mut self.steps[..end] {
            s.done = true;
            s.missed = false;
        }
    }

    /// Untick item `i` and every item after it.
    pub fn untick_from(&mut self, i: usize) {
        let i = i.min(self.steps.len());
        for s in &mut self.steps[i..] {
            s.done = false;
        }
    }

    pub fn next_step(&self) -> Option<&Step> {
        self.next_index(None).map(|i| &self.steps[i])
    }

    /// The item "Next" points at: the first open story item, or, when only optional ones are
    /// left, the first open optional item.
    pub fn next_index(&self, today: Option<(u32, u32)>) -> Option<usize> {
        let open = |s: &Step| s.is_open(today);
        self.steps
            .iter()
            .position(|s| open(s) && !s.optional)
            .or_else(|| self.steps.iter().position(|s| open(s)))
    }

    /// After ticking things off, point Resume at the next item.
    pub fn follow_checklist(&mut self, today: Option<(u32, u32)>) {
        if self.kind == Kind::Video || self.kind == Kind::Sheet {
            return;
        }
        let (anchor, section, page) = match self.next_index(today).map(|i| &self.steps[i]) {
            Some(n) => (n.anchor.clone(), n.section.clone(), n.url.clone()),
            None => return,
        };
        self.resume_page = page;
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
    /// Current in-game date as "M/D", for games with a calendar and missable deadlines.
    pub game_date: String,
    /// Explorer folder chosen by hand; `None` = worked out from the platform.
    pub folder: Option<Folder>,
    /// Box art: "" = looked up automatically (Wikipedia), "none" = no cover, else an image file.
    pub cover: String,
    /// For achievement sync: the game's Steam app id and Xbox title id (found automatically or typed).
    pub steam_app: String,
    pub xbox_title: String,
    /// Your own labels ("100% run", "co-op"...).
    pub tags: Vec<String>,
    /// Time played, in seconds, over every run (not counting a session that's still running).
    pub play_secs: u64,
    /// When the session that's running started.
    pub session_start: Option<i64>,
    /// Earlier playthroughs (New Game+ etc.), oldest first.
    pub runs: Vec<Run>,
    /// The current playthrough's name; empty = "Run 1" (or the next number).
    pub run_name: String,
    /// Cover the names and details of story items you haven't reached.
    pub spoiler_guard: bool,
}

/// A finished (or set aside) playthrough: which items were ticked in each guide.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub name: String,
    /// When it was set aside.
    pub ended: i64,
    /// Time played in this run.
    pub play_secs: u64,
    pub guides: Vec<RunGuide>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunGuide {
    pub guide: u64,
    /// Indexes of the items that were ticked / marked missed.
    pub done: Vec<u32>,
    pub missed: Vec<u32>,
}

impl Run {
    /// (ticked, items) over every guide with a checklist.
    pub fn counts(&self, game: &Game) -> (usize, usize) {
        let total = self.guides.iter().filter_map(|rg| game.guides.iter().find(|g| g.id == rg.guide)).map(|g| g.steps.len()).sum();
        (self.guides.iter().map(|g| g.done.len()).sum(), total)
    }
}

/// "1 h 20 min", "45 min", "under a minute".
pub fn fmt_duration(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, 0) => "under a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// "1:02:03" for a running timer.
pub fn fmt_clock(secs: u64) -> String {
    format!("{}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

impl Game {
    pub fn folder(&self) -> Folder {
        self.folder.unwrap_or_else(|| Folder::of_platform(&self.platform))
    }

    pub fn today(&self) -> Option<(u32, u32)> {
        parse_md(&self.game_date)
    }

    /// The guide holding this game's trophy / achievement list: the one with the most achievement
    /// items (if any).
    pub fn trophy_guide(&self) -> Option<usize> {
        self.guides
            .iter()
            .enumerate()
            .map(|(i, g)| (i, g.steps.iter().filter(|s| s.kind == StepKind::Achievement).count()))
            .filter(|(_, n)| *n > 0)
            .max_by_key(|(_, n)| *n)
            .map(|(i, _)| i)
    }

    pub fn has_deadlines(&self) -> bool {
        self.guides.iter().flat_map(|g| &g.steps).any(|s| !s.deadline.is_empty())
    }

    pub fn progress(&self) -> f32 {
        if self.status == Status::Completed {
            return 1.0;
        }
        // Only guides you track progress in count; plain reference links don't drag it down.
        let tracked: Vec<&Guide> = self.guides.iter().filter(|g| !g.steps.is_empty() || g.done || g.percent > 0.0).collect();
        if tracked.is_empty() {
            return 0.0;
        }
        tracked.iter().map(|g| g.progress()).sum::<f32>() / tracked.len() as f32
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

    /// Time played, including a session that's running (as of `at`).
    pub fn played(&self, at: i64) -> u64 {
        self.play_secs + self.session_start.map_or(0, |s| (at - s).max(0) as u64)
    }

    /// Starts the play timer (if it isn't running).
    pub fn start_session(&mut self, at: i64) {
        if self.session_start.is_none() {
            self.session_start = Some(at);
        }
        self.last_played = Some(at);
    }

    /// Stops the play timer. Returns how long the session was.
    pub fn stop_session(&mut self, at: i64) -> Option<u64> {
        let start = self.session_start.take()?;
        let secs = (at - start).max(0) as u64;
        self.play_secs += secs;
        self.last_played = Some(at);
        Some(secs)
    }

    /// Time played in the current run.
    pub fn run_played(&self, at: i64) -> u64 {
        self.played(at).saturating_sub(self.runs.iter().map(|r| r.play_secs).sum())
    }

    /// How long the rest should take at your pace so far in this run: time played x what's left /
    /// what's done. Needs half an hour of play and some progress to go on.
    pub fn estimate_left(&self, at: i64) -> Option<u64> {
        let played = self.run_played(at);
        let p = self.progress();
        if played < 1800 || !(0.03..0.995).contains(&p) || self.status == Status::Completed {
            return None;
        }
        Some((played as f64 * (1.0 - p as f64) / p as f64) as u64)
    }

    /// The current run's name.
    pub fn run_label(&self) -> String {
        if self.run_name.trim().is_empty() {
            format!("Run {}", self.runs.len() + 1)
        } else {
            self.run_name.trim().to_string()
        }
    }

    /// Snapshot of the current run's ticks.
    fn snapshot_run(&self, at: i64) -> Run {
        Run {
            name: self.run_label(),
            ended: at,
            play_secs: self.run_played(at),
            guides: self
                .guides
                .iter()
                .filter(|g| !g.steps.is_empty())
                .map(|g| RunGuide {
                    guide: g.id,
                    done: g.steps.iter().enumerate().filter(|(_, s)| s.done).map(|(i, _)| i as u32).collect(),
                    missed: g.steps.iter().enumerate().filter(|(_, s)| s.missed).map(|(i, _)| i as u32).collect(),
                })
                .collect(),
        }
    }

    /// New Game+ (or a fresh start): files the current run away, then unticks every guide except
    /// the ones in `keep`. With `keep_achievements`, achievement items stay ticked everywhere
    /// (trophies you've earned stay earned).
    pub fn new_run(&mut self, name: &str, keep: &[u64], keep_achievements: bool, at: i64) {
        let snap = self.snapshot_run(at);
        self.runs.push(snap);
        self.run_name = name.trim().to_string();
        let today = self.today();
        for g in &mut self.guides {
            if keep.contains(&g.id) {
                continue;
            }
            for s in &mut g.steps {
                if keep_achievements && s.kind == StepKind::Achievement {
                    continue;
                }
                s.done = false;
                s.missed = false;
            }
            g.done = false;
            g.follow_checklist(today);
        }
    }

    /// Goes back to an earlier run: the current one is filed away in its place.
    pub fn switch_run(&mut self, i: usize, at: i64) {
        if i >= self.runs.len() {
            return;
        }
        // (the current run's time is what's played beyond every filed run, the old one included)
        let current = self.snapshot_run(at);
        let old = self.runs.remove(i);
        let today = self.today();
        for g in &mut self.guides {
            let Some(rg) = old.guides.iter().find(|r| r.guide == g.id) else { continue };
            for (j, s) in g.steps.iter_mut().enumerate() {
                s.done = rg.done.contains(&(j as u32));
                s.missed = rg.missed.contains(&(j as u32));
            }
            g.follow_checklist(today);
        }
        self.run_name = old.name;
        self.runs.insert(i, current);
    }
}

// ---------------------------------------------------------------------------------------------
// Guide text for the reader, kept beside the library (not in it: pages can be big).

pub fn page_path(dir: &Path, guide: u64) -> PathBuf {
    dir.join(format!("{guide}.txt"))
}

pub fn load_page(dir: &Path, guide: u64) -> Option<String> {
    std::fs::read_to_string(page_path(dir, guide)).ok().filter(|t| !t.trim().is_empty())
}

pub fn save_page(dir: &Path, guide: u64, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(page_path(dir, guide), text)
}

pub fn pages_dir() -> PathBuf {
    data_dir().join("pages")
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

/// How many daily backups to keep.
pub const BACKUP_DAYS: usize = 14;

fn backup_dir(library: &Path) -> PathBuf {
    library.parent().unwrap_or(Path::new(".")).join("backups")
}

/// Before the first save of each day, copies the library to `backups/library-YYYY-MM-DD.json`,
/// keeping the newest `BACKUP_DAYS`.
pub fn daily_backup(library: &Path) -> std::io::Result<()> {
    if !library.exists() {
        return Ok(());
    }
    let dir = backup_dir(library);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let file = dir.join(format!("library-{today}.json"));
    if !file.exists() {
        std::fs::create_dir_all(&dir)?;
        std::fs::copy(library, &file)?;
    }
    let mut all = list_backups(library);
    while all.len() > BACKUP_DAYS {
        if let Some((oldest, _)) = all.pop() {
            let _ = std::fs::remove_file(oldest);
        }
    }
    Ok(())
}

/// Daily backups, newest first: (file, "2026-11-15").
pub fn list_backups(library: &Path) -> Vec<(PathBuf, String)> {
    let Ok(rd) = std::fs::read_dir(backup_dir(library)) else { return Vec::new() };
    let mut out: Vec<(PathBuf, String)> = rd
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            let date = name.strip_prefix("library-")?.strip_suffix(".json")?.to_string();
            Some((p, date))
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    fn st(text: &str, optional: bool, deadline: &str) -> Step {
        Step { text: text.into(), optional, deadline: deadline.into(), ..Default::default() }
    }

    #[test]
    fn daily_backups_rotate() {
        let dir = std::env::temp_dir().join(format!("jagt-backup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        let lib = dir.join("library.json");
        std::fs::write(&lib, "{}").unwrap();
        // 20 old backups: rotation keeps the newest 14, including today's
        for d in 1..=20 {
            std::fs::write(dir.join("backups").join(format!("library-2020-01-{d:02}.json")), "{}").unwrap();
        }
        daily_backup(&lib).unwrap();
        let all = list_backups(&lib);
        assert_eq!(all.len(), BACKUP_DAYS);
        assert_eq!(all[0].1, chrono::Local::now().format("%Y-%m-%d").to_string(), "today's backup is newest");
        assert_eq!(all.last().unwrap().1, "2020-01-08", "the oldest were removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn platform_folders() {
        use Folder::*;
        for (p, f) in [
            ("PS5", Console),
            ("PS4", Console),
            ("PlayStation 2", Console),
            ("Xbox Series X", Console),
            ("Wii U", Console),
            ("PSP", Console),
            ("PS Vita", Console),
            ("Switch 2", Console),
            ("Nintendo Switch", Console),
            ("3DS", Console),
            ("Game Boy Advance", Console),
            ("PC (Steam)", Pc),
            ("PC (Browser)", Pc),
            ("Windows", Pc),
            ("Steam", Pc),
            ("ROG Ally", Pc),
            ("Steam Deck", Linux),
            ("Linux", Linux),
            ("Android", Mobile),
            ("iPhone", Mobile),
            ("", Other),
            ("Arcade cabinet", Other),
        ] {
            assert_eq!(Folder::of_platform(p), f, "{p}");
        }
        let mut g = Game { platform: "PS5".into(), ..Default::default() };
        assert_eq!(g.folder(), Console);
        g.folder = Some(Pc);
        assert_eq!(g.folder(), Pc);
    }

    fn game_with(steps: usize) -> Game {
        let mut g = Game { id: 1, ..Default::default() };
        let mut a = Guide { id: 10, ..Default::default() };
        let mut t = Guide { id: 11, ..Default::default() };
        for i in 0..steps {
            a.steps.push(Step::task(format!("chapter {i}")));
            t.steps.push(Step { kind: StepKind::Achievement, ..Step::task(format!("trophy {i}")) });
        }
        g.guides = vec![a, t];
        g
    }

    #[test]
    fn new_game_plus_and_back() {
        let mut g = game_with(4);
        g.guides[0].steps[0].done = true;
        g.guides[0].steps[1].done = true;
        g.guides[0].steps[3].missed = true;
        g.guides[1].steps[2].done = true;
        g.play_secs = 3600;
        // NG+: the walkthrough resets, achievements stay earned
        g.new_run("NG+", &[], true, 100);
        assert_eq!(g.runs.len(), 1);
        assert_eq!(g.runs[0].name, "Run 1");
        assert_eq!(g.runs[0].play_secs, 3600);
        assert_eq!(g.runs[0].counts(&g), (3, 8));
        assert!(g.guides[0].steps.iter().all(|s| !s.done && !s.missed));
        assert!(g.guides[1].steps[2].done, "achievements kept");
        assert_eq!(g.run_label(), "NG+");
        assert_eq!(g.run_played(100), 0);
        g.play_secs += 600;
        g.guides[0].steps[2].done = true;
        // back to run 1: its ticks return, NG+ is filed away with its own time
        g.switch_run(0, 200);
        assert_eq!(g.run_label(), "Run 1");
        assert!(g.guides[0].steps[0].done && g.guides[0].steps[1].done && !g.guides[0].steps[2].done && g.guides[0].steps[3].missed);
        assert_eq!(g.runs[0].name, "NG+");
        assert_eq!(g.runs[0].play_secs, 600);
        assert_eq!(g.run_played(200), 3600);
        // a guide you choose to keep isn't unticked
        g.new_run("Run 3", &[10], false, 300);
        assert!(g.guides[0].steps[0].done);
        assert!(!g.guides[1].steps[2].done);
    }

    #[test]
    fn play_time_and_estimate() {
        let mut g = game_with(10);
        g.start_session(1000);
        g.start_session(2000); // already running: keeps the first start
        assert_eq!(g.played(1600), 600);
        assert_eq!(g.stop_session(4600), Some(3600));
        assert_eq!(g.stop_session(5000), None);
        assert_eq!(g.play_secs, 3600);
        assert_eq!(g.estimate_left(5000), None, "no progress yet");
        // 1 h for a quarter of it: about 3 h to go
        for i in 0..5 {
            g.guides[0].steps[i].done = true;
        }
        assert_eq!(g.progress(), 0.25);
        assert_eq!(g.estimate_left(5000), Some(3 * 3600));
        assert_eq!(fmt_duration(4980), "1 h 23 min");
        assert_eq!(fmt_duration(30), "under a minute");
        assert_eq!(fmt_clock(3723), "1:02:03");
    }

    #[test]
    fn missable_items() {
        assert!(st("quest", true, "9/25").is_missable());
        assert!(Step { detail: "Missable! Talk to her before the bridge.".into(), ..Step::task("Letter") }.is_missable());
        assert!(!Step::task("Chapter 1").is_missable());
    }

    #[test]
    fn deadlines_and_next() {
        assert_eq!(parse_md("9/25"), Some((9, 25)));
        assert_eq!(parse_md("13/1"), None);
        let g = Guide {
            steps: vec![st("old quest", true, "2/28"), st("battle", false, ""), st("soon quest", true, "4/10"), st("later quest", true, "9/1")],
            ..Default::default()
        };
        let today = Some((4, 2));
        assert_eq!(g.steps[0].state(today), StepState::Missed);
        assert_eq!(g.steps[2].state(today), StepState::DueSoon);
        assert_eq!(g.steps[3].state(today), StepState::Open);
        assert_eq!(g.steps[0].state(None), StepState::Open);
        // story first, even though optional items come earlier in the list
        assert_eq!(g.next_index(today), Some(1));
        let mut g2 = g.clone();
        g2.steps[1].done = true;
        // then the first optional item that can still be done
        assert_eq!(g2.next_index(today), Some(2));
    }
}
