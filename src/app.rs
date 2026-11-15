use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

use egui::{Align, Align2, Color32, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};

use crate::icons;
use crate::links;
use crate::accounts::{self, Source};
use crate::checklist;
use crate::hotkeys::{self, HotAction, Hotkeys};
use crate::model::{self, Folder, Game, Guide, Kind, Library, LogEntry, Status, Step, StepKind, StepState};
use crate::online::{self, Done, Job, Online};
use crate::theme::*;

const GAME_ROW_H: f32 = 48.0;
const CARD_BG: Color32 = Color32::from_rgb(0x22, 0x20, 0x3a);
const HOVER_BG: Color32 = Color32::from_rgb(0x2a, 0x22, 0x50);
const GREEN: Color32 = Color32::from_rgb(0x8C, 0xF0, 0x8C);

#[derive(Default)]
pub struct Options {
    pub fresh: bool,
    pub open_settings: bool,
}

/// UI preferences (the library itself lives in its own library.json).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    filter: Option<Status>,
    selected: Option<u64>,
    online: bool,
    glow: bool,
    /// Drifting stars. Off = a still background and far fewer redraws.
    animate: bool,
    /// Games panel: folders (true) or one flat list.
    folder_view: bool,
    /// Explorer folders the user closed ("Console", "Console/ps5"...).
    closed_folders: Vec<String>,
    /// The explorer window's view: large icons or details.
    ex_layout: ExplorerLayout,
    /// One-line checklist items.
    compact_rows: bool,
    /// Which windows get the space.
    view: ViewMode,
    /// Achievement sync accounts (kept only in the local settings file).
    steam_key: String,
    steam_profile: String,
    steam_id: String,
    steam_name: String,
    xbox_key: String,
    xbox_tag: String,
    /// Reopen the explorer where it was left.
    explorer_open: bool,
    ex_path: Vec<String>,
    window_size: Option<[f32; 2]>,
    maximized: bool,
    /// The small always-on-top "next item" window.
    mini_open: bool,
    /// Games list: only games with this tag.
    tag_filter: Option<String>,
    /// The guide's text beside the checklist.
    reader_open: bool,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { filter: None, selected: None, online: true, glow: true, animate: true, folder_view: true, closed_folders: Vec::new(), ex_layout: ExplorerLayout::Icons, compact_rows: false, view: ViewMode::All, steam_key: String::new(), steam_profile: String::new(), steam_id: String::new(), steam_name: String::new(), xbox_key: String::new(), xbox_tag: String::new(), explorer_open: false, ex_path: Vec::new(), window_size: None, maximized: false, mini_open: false, tag_filter: None, reader_open: false }
    }
}

enum Fav {
    Loading,
    Ready(TextureHandle),
    Missing,
}

struct NewGame {
    title: String,
    platform: String,
    status: Status,
    focus: bool,
}

/// Which of the game's windows get the space: all of them, or just one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
enum ViewMode {
    #[default]
    All,
    Game,
    Guides,
    Checklist,
    /// Where I Left Off + Session Log.
    Notes,
}

impl ViewMode {
    const ALL: [ViewMode; 5] = [ViewMode::All, ViewMode::Game, ViewMode::Guides, ViewMode::Checklist, ViewMode::Notes];

    fn label(self) -> &'static str {
        match self {
            ViewMode::All => "All",
            ViewMode::Game => "Game",
            ViewMode::Guides => "Guides",
            ViewMode::Checklist => "Checklist",
            ViewMode::Notes => "Notes",
        }
    }
}

/// What a guide card asks the app to do once the list has been drawn.
enum CardAct {
    Open,
    Up,
    Down,
    Remove,
    Refetch,
    BulkSteps,
    ShowChecklist,
    ShowInFolder,
    /// Open the Wayback Machine's copy of a page that's gone.
    Archive,
    CheckLink,
}

/// What a checklist row asks for.
enum RowAct {
    Toggle,
    /// Tick every item above this one (list order).
    TickBefore,
    /// Untick this item and every one below it.
    UntickFrom,
    /// Open or close a compact row's details.
    Details,
    Missed,
    Optional,
    Find,
    Delete,
    Up,
    Down,
    /// Show a spoiler-guarded item.
    Reveal,
    /// Edit the item's note.
    Note,
    /// Attach / remove / open the item's image.
    Image,
    NoImage,
    OpenImage,
}

/// How a checklist row is drawn.
#[derive(Clone, Copy)]
struct RowLook {
    state: StepState,
    is_next: bool,
    can_find: bool,
    compact: bool,
    open: bool,
    cursor: bool,
    /// Spoiler guard: the name and details are covered.
    veiled: bool,
    thumb: Option<(egui::TextureId, Vec2)>,
}

pub struct App {
    lib: Library,
    lib_path: PathBuf,
    saved_hash: u64,
    last_hash: u64,
    changed_at: f64,

    selected: Option<u64>,
    filter: Option<Status>,
    search: String,
    online_on: bool,
    glow: bool,
    animate: bool,
    folder_view: bool,
    closed_folders: Vec<String>,
    ex_layout: ExplorerLayout,
    compact_rows: bool,
    /// All windows, or just one of them using the whole space (Focus = just the Checklist).
    view: ViewMode,
    steam_key: String,
    steam_profile: String,
    steam_id: String,
    steam_name: String,
    xbox_key: String,
    xbox_tag: String,
    /// Result of the last account check.
    account_note: String,
    /// The "Sync achievements" window, for this game.
    sync_window: Option<u64>,
    sync_note: String,
    /// The Trophies dashboard is showing.
    trophies_open: bool,
    /// Compact rows whose details are opened, by (guide, item).
    open_rows: HashSet<(u64, usize)>,
    /// The explorer window is showing (instead of the selected game).
    explorer_open: bool,
    /// Where the explorer is: [] = top, ["Console"], ["Console", "PS5"].
    ex_path: Vec<String>,
    ex_back: Vec<Vec<String>>,
    ex_fwd: Vec<Vec<String>>,
    ex_sel: Option<ExplorerItem>,
    /// Checklist rows' last drawn height, keyed by (guide, item), with a fingerprint of what was
    /// drawn. Rows scrolled out of view reuse it instead of being laid out again.
    row_heights: HashMap<(u64, usize), (u64, f32)>,
    online: Online,
    favicons: HashMap<String, Fav>,
    /// Box art by game id (same states as favicons).
    covers: HashMap<u64, Fav>,

    expanded: HashSet<u64>,
    new_url: String,
    new_log: String,
    time_bufs: HashMap<u64, String>,
    step_bufs: HashMap<u64, String>,
    bulk: Option<(u64, String)>,
    bulk_replace: bool,
    bulk_fetching: bool,
    bulk_error: Option<String>,
    bulk_headings: bool,
    /// (hash of text + mode, parsed items) for the paste window's preview.
    bulk_cache: Option<(u64, Vec<Step>)>,
    /// "Tick from a list": (guide, pasted text, mode).
    tick_list: Option<(u64, String, TickMode)>,
    /// Keyboard highlight in the checklist: (guide, item).
    list_cursor: Option<(u64, usize)>,
    /// "Search everything" window: its text.
    search_all: Option<String>,
    active_list: Option<u64>,
    list_kind: Option<StepKind>,
    /// Some(false) = story items only, Some(true) = optional only.
    list_scope: Option<bool>,
    list_section: Option<String>,
    left_view: bool,
    hide_done: bool,
    list_search: String,
    collapsed: HashSet<(u64, String)>,
    scroll_to_next: bool,
    file_mtime: Option<std::time::SystemTime>,
    undo: Vec<Library>,
    redo: Vec<Library>,
    /// The library as of the last change, pushed onto `undo` when the next edit begins.
    snapshot: Library,
    last_file_check: f64,
    add_game: Option<NewGame>,
    show_settings: bool,

    status: Option<(String, f64)>,
    started: Instant,
    window_size: Option<Vec2>,
    maximized: bool,
    restore_window: Option<(Option<[f32; 2]>, bool)>,
    restore_wait: u32,
    persist: bool,

    /// The mini "next item" window is open.
    mini_open: bool,
    /// System-wide hotkeys (None in tests, or if Windows refused them).
    hotkeys: Option<Hotkeys>,
    /// What the hotkey ticked, newest last, for its undo: (game, guide, item).
    hotkey_ticks: Vec<(u64, u64, usize)>,
    tag_filter: Option<String>,
    new_tag: String,
    /// "New run" window: (game, name, guides to keep ticked, keep achievements).
    run_window: Option<(u64, String, HashSet<u64>, bool)>,
    /// The Missables dashboard is showing.
    missables_open: bool,
    /// Missables: include finished / dropped games.
    missables_all: bool,
    /// Spoiler-guarded items shown anyway, by (guide, item).
    revealed: HashSet<(u64, usize)>,
    /// Editing an item's note: (guide, item, text).
    note_edit: Option<(u64, usize, String)>,
    /// Item images, by file path.
    thumbs: HashMap<String, Fav>,
    /// The guide's text is shown beside the checklist.
    reader_open: bool,
    reader: Option<Reader>,
    /// Waiting for this guide's page text, for the reader.
    reader_fetching: Option<u64>,
    reader_find: String,
    /// Where guide texts are kept (jagt-data/pages).
    pages_dir: PathBuf,
    /// Link checks still running.
    link_checks: usize,
    link_gone: usize,
    /// Scroll the checklist to this item on the next frame: (guide, item).
    scroll_to_item: Option<(u64, usize)>,
}

/// The reader: a guide's text, split into lines, with remembered heights so only what's on
/// screen is laid out.
struct Reader {
    guide: u64,
    lines: Vec<String>,
    /// Laid-out height of each line at `width` (an estimate until it has been drawn).
    heights: Vec<f32>,
    width: f32,
    /// The line the current item is on, and what it was worked out for.
    target: Option<usize>,
    target_key: String,
    /// Scroll to this line on the next frame.
    jump: Option<usize>,
    /// Search matches and the one on show.
    found: Vec<usize>,
    found_at: usize,
    found_for: String,
}

fn hash_lib(lib: &Library) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(lib).unwrap_or_default().hash(&mut h);
    h.finish()
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        install(&cc.egui_ctx);
        let saved: Saved = if opts.fresh {
            Saved::default()
        } else {
            cc.storage.and_then(|s| eframe::get_value(s, "jagt")).unwrap_or_default()
        };
        let lib_path = model::library_path();
        let (lib, problem) = model::load(&lib_path);
        let mut app = Self::build(&cc.egui_ctx, saved, lib, lib_path, !opts.fresh, opts.open_settings);
        app.hotkeys = Hotkeys::new(cc.egui_ctx.clone());
        if let Some(msg) = problem {
            app.set_status_for(msg, 12.0);
        }
        app
    }

    fn build(ctx: &egui::Context, saved: Saved, lib: Library, lib_path: PathBuf, persist: bool, open_settings: bool) -> Self {
        set_glow(saved.glow);
        let h = hash_lib(&lib);
        let mut app = App {
            lib,
            lib_path,
            saved_hash: h,
            undo: Vec::new(),
            redo: Vec::new(),
            snapshot: Library::default(),
            last_hash: h,
            changed_at: 0.0,
            selected: saved.selected,
            filter: saved.filter,
            search: String::new(),
            online_on: saved.online,
            glow: saved.glow,
            animate: saved.animate,
            folder_view: saved.folder_view,
            closed_folders: saved.closed_folders,
            ex_layout: saved.ex_layout,
            compact_rows: saved.compact_rows,
            view: saved.view,
            steam_key: saved.steam_key,
            steam_profile: saved.steam_profile,
            steam_id: saved.steam_id,
            steam_name: saved.steam_name,
            xbox_key: saved.xbox_key,
            xbox_tag: saved.xbox_tag,
            account_note: String::new(),
            sync_window: None,
            sync_note: String::new(),
            trophies_open: false,
            open_rows: HashSet::new(),
            explorer_open: saved.explorer_open,
            ex_path: saved.ex_path,
            ex_back: Vec::new(),
            ex_fwd: Vec::new(),
            ex_sel: None,
            row_heights: HashMap::new(),
            online: Online::new(ctx.clone()),
            favicons: HashMap::new(),
            covers: HashMap::new(),
            expanded: HashSet::new(),
            new_url: String::new(),
            new_log: String::new(),
            time_bufs: HashMap::new(),
            step_bufs: HashMap::new(),
            bulk: None,
            bulk_replace: false,
            bulk_fetching: false,
            bulk_error: None,
            bulk_headings: false,
            bulk_cache: None,
            tick_list: None,
            list_cursor: None,
            search_all: None,
            active_list: None,
            list_kind: None,
            list_scope: None,
            list_section: None,
            left_view: false,
            hide_done: false,
            list_search: String::new(),
            collapsed: HashSet::new(),
            scroll_to_next: false,
            file_mtime: None,
            last_file_check: 0.0,
            add_game: None,
            show_settings: open_settings,
            status: None,
            started: Instant::now(),
            window_size: saved.window_size.map(Vec2::from),
            maximized: saved.maximized,
            restore_window: Some((saved.window_size, saved.maximized)),
            restore_wait: 0,
            persist,
            mini_open: saved.mini_open,
            hotkeys: None,
            hotkey_ticks: Vec::new(),
            tag_filter: saved.tag_filter,
            new_tag: String::new(),
            run_window: None,
            missables_open: false,
            missables_all: false,
            revealed: HashSet::new(),
            note_edit: None,
            thumbs: HashMap::new(),
            reader_open: saved.reader_open,
            reader: None,
            reader_fetching: None,
            reader_find: String::new(),
            pages_dir: if persist { model::pages_dir() } else { std::env::temp_dir().join(format!("jagt-test-pages-{}", std::process::id())) },
            link_checks: 0,
            link_gone: 0,
            scroll_to_item: None,
        };
        // A session left running by a crash: keep it if it's plausible, else drop it.
        let at = model::now();
        for g in &mut app.lib.games {
            if g.session_start.is_some_and(|s| at - s > 12 * 3600) {
                g.session_start = None;
            }
        }
        if app.tag_filter.as_ref().is_some_and(|t| !app.lib.games.iter().any(|g| g.tags.contains(t))) {
            app.tag_filter = None;
        }
        app.file_mtime = mtime(&app.lib_path);
        app.snapshot = app.lib.clone();
        if app.selected.is_none_or(|id| !app.lib.games.iter().any(|g| g.id == id)) {
            app.selected = app.sorted_ids().first().copied();
        }
        app
    }

    fn time(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn set_status(&mut self, s: impl Into<String>) {
        self.set_status_for(s, 2.5);
    }

    fn set_status_for(&mut self, s: impl Into<String>, secs: f64) {
        self.status = Some((s.into(), self.time() + secs));
    }

    fn game(&self) -> Option<&Game> {
        let id = self.selected?;
        self.lib.games.iter().find(|g| g.id == id)
    }

    /// Games in list order: playing first, then alphabetical.
    fn sorted_ids(&self) -> Vec<u64> {
        let q = self.search.to_lowercase();
        let mut v: Vec<&Game> = self
            .lib
            .games
            .iter()
            .filter(|g| self.filter.is_none_or(|f| g.status == f))
            .filter(|g| self.tag_filter.as_ref().is_none_or(|t| g.tags.contains(t)))
            .filter(|g| q.is_empty() || g.title.to_lowercase().contains(&q) || g.platform.to_lowercase().contains(&q) || g.tags.iter().any(|t| t.to_lowercase().contains(&q)))
            .collect();
        // A fixed order (status, then title) so rows never shuffle while you use them.
        v.sort_by(|a, b| a.status.rank().cmp(&b.status.rank()).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())).then(a.id.cmp(&b.id)));
        v.iter().map(|g| g.id).collect()
    }

    // -----------------------------------------------------------------------------------------
    // Actions

    fn create_game(&mut self, title: String, platform: String, status: Status) {
        let id = self.lib.new_id();
        let title = if title.trim().is_empty() { "Untitled game".to_string() } else { title.trim().to_string() };
        self.set_status(format!("Added {title}"));
        self.lib.games.push(Game { id, title, platform: platform.trim().to_string(), status, added: model::now(), ..Default::default() });
        self.selected = Some(id);
        if self.filter.is_some_and(|f| f != status) {
            self.filter = None;
        }
    }

    fn add_guide(&mut self, input: &str) -> bool {
        let Some(game_id) = self.selected else {
            self.set_status("Pick or add a game first");
            return false;
        };
        let Some(url) = links::normalize(input) else {
            self.set_status("That doesn't look like a link");
            return false;
        };
        let id = self.lib.new_id();
        let mut guide = Guide::new(id, url.clone());
        let known_title = links::unwrap_suspended(input).and_then(|(_, t)| t);
        if let Some(t) = &known_title {
            guide.title = t.clone();
            guide.auto_title = false;
        }
        let kind = guide.kind;
        let Some(game) = self.lib.game_mut(game_id) else { return false };
        if game.guides.iter().any(|g| g.url == url) {
            self.set_status("That guide is already on this game");
            return false;
        }
        game.guides.push(guide);
        if game.last_guide.is_none() {
            game.last_guide = Some(id);
        }
        if self.online_on && known_title.is_none() && !links::is_local(&url) {
            self.online.send(Job::Title { game: game_id, guide: id, url });
        }
        self.set_status(format!("Added {}", kind.label().to_lowercase()));
        true
    }

    /// Adds files (and every guide file inside folders, recursively) to the selected game.
    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        if self.selected.is_none() {
            self.set_status("Pick or add a game first, then add its guide files");
            return;
        }
        let mut files = Vec::new();
        for p in &paths {
            collect_guide_files(p, &mut files, 0);
        }
        if files.is_empty() {
            self.set_status("No guide files there (PDF, text, web pages, spreadsheets, images, documents)");
            return;
        }
        let total = files.len();
        let added = files.iter().filter(|f| self.add_guide(&links::file_url(f))).count();
        if total > 1 || added == 0 {
            self.set_status(format!("Added {added} of {total} files"));
        }
    }

    fn pick_files(&mut self) {
        let types: Vec<&str> = links::GUIDE_FILE_TYPES.to_vec();
        if let Some(files) = rfd::FileDialog::new().set_title("Add guide files").add_filter("Guide files", &types).add_filter("All files", &["*"]).pick_files() {
            self.add_paths(files);
        }
    }

    fn pick_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Add every guide file in a folder").pick_folder() {
            self.add_paths(vec![dir]);
        }
    }

    fn open_guide(&mut self, game_id: u64, guide_id: u64) {
        let Some(game) = self.lib.game_mut(game_id) else { return };
        let Some(g) = game.guides.iter_mut().find(|g| g.id == guide_id) else { return };
        let url = g.resume_url();
        g.last_opened = Some(model::now());
        let spot = g.spot();
        game.last_guide = Some(guide_id);
        game.touch();
        if let Some(path) = links::local_path(&url) {
            if !path.exists() {
                self.set_status_for(format!("File not found: {}", path.display()), 6.0);
                return;
            }
            match open::that_detached(&path) {
                Ok(()) => self.set_status("Opened the file"),
                Err(e) => self.set_status_for(format!("Couldn't open the file: {e}"), 6.0),
            }
            return;
        }
        match open::that_detached(&url) {
            Ok(()) => self.set_status(if spot.is_empty() { "Opened in your browser".to_string() } else { format!("Opened {spot}") }),
            Err(e) => self.set_status_for(format!("Couldn't open the browser: {e}"), 6.0),
        }
    }

    fn continue_game(&mut self) {
        let Some(game) = self.game() else { return };
        let (gid, guide) = (game.id, game.continue_guide().map(|g| g.id));
        match guide {
            Some(id) => {
                self.open_guide(gid, id);
                self.start_playing(gid);
            }
            None => self.set_status("Add a guide to this game first"),
        }
    }

    /// Starts this game's play timer, stopping any other game's.
    fn start_playing(&mut self, gid: u64) {
        let at = model::now();
        let others: Vec<u64> = self.lib.games.iter().filter(|g| g.id != gid && g.session_start.is_some()).map(|g| g.id).collect();
        for o in others {
            self.stop_playing(o);
        }
        if let Some(g) = self.lib.game_mut(gid) {
            g.start_session(at);
        }
    }

    /// Stops a game's play timer and logs the session.
    fn stop_playing(&mut self, gid: u64) {
        let Some(g) = self.lib.game_mut(gid) else { return };
        if let Some(secs) = g.stop_session(model::now()) {
            if secs >= 60 {
                g.log.push(LogEntry { at: model::now(), text: format!("Played {}", model::fmt_duration(secs)) });
            }
            let msg = format!("Stopped the timer: {} this session", model::fmt_duration(secs));
            self.set_status(msg);
        }
    }

    /// The game whose timer is running.
    fn playing(&self) -> Option<&Game> {
        self.lib.games.iter().find(|g| g.session_start.is_some())
    }

    /// The guide the mini window and the hotkeys work on: the checklist on show for the selected
    /// game, else the one Continue would open.
    fn current_list(&self) -> Option<(u64, u64)> {
        let g = self.game()?;
        let on_show = self.active_list.filter(|id| g.guides.iter().any(|x| x.id == *id && !x.steps.is_empty()));
        let id = on_show
            .or_else(|| g.continue_guide().filter(|x| !x.steps.is_empty()).map(|x| x.id))
            .or_else(|| g.guides.iter().find(|x| !x.steps.is_empty()).map(|x| x.id))?;
        Some((g.id, id))
    }

    /// Ticks the next item of the current checklist. Returns what was ticked.
    fn tick_next(&mut self) -> Option<String> {
        let (gid, lid) = self.current_list()?;
        let game = self.lib.game_mut(gid)?;
        let today = game.today();
        let guide = game.guides.iter_mut().find(|g| g.id == lid)?;
        let i = guide.next_index(today)?;
        guide.steps[i].done = true;
        guide.steps[i].missed = false;
        let text = guide.steps[i].text.clone();
        guide.follow_checklist(today);
        game.last_guide = Some(lid);
        game.touch();
        self.hotkey_ticks.push((gid, lid, i));
        self.set_status(format!("Ticked {text}"));
        Some(text)
    }

    /// Unticks what `tick_next` last ticked.
    fn untick_last(&mut self) {
        let Some((gid, lid, i)) = self.hotkey_ticks.pop() else {
            self.set_status("Nothing to untick");
            return;
        };
        let Some(game) = self.lib.game_mut(gid) else { return };
        let today = game.today();
        let Some(guide) = game.guides.iter_mut().find(|g| g.id == lid) else { return };
        if let Some(s) = guide.steps.get_mut(i) {
            s.done = false;
            let text = s.text.clone();
            guide.follow_checklist(today);
            self.set_status(format!("Unticked {text}"));
        }
    }

    /// Opens a game (and a guide's checklist, at an item) from a dashboard or search.
    fn go_to(&mut self, game: u64, guide: Option<u64>, item: Option<usize>, search: String) {
        self.selected = Some(game);
        self.explorer_open = false;
        self.trophies_open = false;
        self.missables_open = false;
        if matches!(self.view, ViewMode::Game | ViewMode::Guides | ViewMode::Notes) {
            self.view = ViewMode::All;
        }
        if let Some(gid) = guide {
            self.active_list = Some(gid);
            self.left_view = false;
            self.list_section = None;
            self.list_kind = None;
            self.list_scope = None;
            self.hide_done = false;
            self.list_search = search;
            self.list_cursor = item.map(|i| (gid, i));
            if let Some(i) = item {
                // make sure its area is open, and the reader follows it
                let sec = self.lib.games.iter().flat_map(|g| &g.guides).find(|g| g.id == gid).and_then(|g| g.steps.get(i)).map(|s| s.section.clone());
                if let Some(sec) = sec {
                    self.collapsed.remove(&(gid, sec));
                }
                self.scroll_to_item = Some((gid, i));
            }
        }
    }

    /// Checks whether guide pages still exist (all games, or one).
    fn check_links(&mut self, game: Option<u64>) {
        if !self.online_on {
            self.set_status("Turn Online on to check links");
            return;
        }
        let mut n = 0;
        let mut jobs = Vec::new();
        for g in self.lib.games.iter_mut().filter(|g| game.is_none_or(|id| g.id == id)) {
            for gd in &mut g.guides {
                if let Some(path) = links::local_path(&gd.url) {
                    gd.dead = !path.exists();
                    continue;
                }
                if gd.url.starts_with("http") {
                    jobs.push(Job::CheckLink { game: g.id, guide: gd.id, url: gd.url.clone() });
                    n += 1;
                }
            }
        }
        for j in jobs {
            self.online.send(j);
        }
        self.link_checks += n;
        self.link_gone = 0;
        self.set_status(format!("Checking {n} links\u{2026}"));
    }

    fn pump(&mut self, ctx: &egui::Context) -> bool {
        let mut any = false;
        while let Ok(done) = self.online.rx.try_recv() {
            any = true;
            match done {
                Done::Title { game, guide, title } => {
                    if let Some(t) = title {
                        if let Some(g) = self.lib.game_mut(game).and_then(|gm| gm.guides.iter_mut().find(|g| g.id == guide)) {
                            if g.auto_title {
                                g.title = t;
                                g.auto_title = false;
                            }
                        }
                    }
                }
                Done::Favicon { host, image } => {
                    let fav = match image {
                        Some(img) => Fav::Ready(ctx.load_texture(format!("fav:{host}"), img, egui::TextureOptions::LINEAR)),
                        None => Fav::Missing,
                    };
                    self.favicons.insert(host, fav);
                }
                Done::Cover { game, image } => {
                    let c = match image {
                        Some(img) => Fav::Ready(ctx.load_texture(format!("cover:{game}"), img, egui::TextureOptions::LINEAR)),
                        None => Fav::Missing,
                    };
                    self.covers.insert(game, c);
                }
                Done::Account { source, result } => match (source, result) {
                    (Source::Steam, Ok((id, name))) => {
                        self.account_note = format!("Steam: signed in as {name}");
                        self.steam_id = id;
                        self.steam_name = name;
                    }
                    (Source::Xbox, Ok((_, tag))) => {
                        self.account_note = format!("Xbox: signed in as {tag}");
                        self.xbox_tag = tag;
                    }
                    (s, Err(e)) => self.account_note = format!("{}: {e}", s.label()),
                },
                Done::FoundGame { game, source, result } => match result {
                    Ok((id, name)) => {
                        if let Some(g) = self.lib.game_mut(game) {
                            match source {
                                Source::Steam => g.steam_app = id.clone(),
                                Source::Xbox => g.xbox_title = id.clone(),
                            }
                        }
                        self.sync_note = format!("{}: found \"{name}\" ({id}). Check it's the right game, then Sync.", source.label());
                    }
                    Err(e) => self.sync_note = format!("{}: {e}", source.label()),
                },
                Done::Achievements { game, source, result } => {
                    self.sync_note = match result {
                        Ok(list) => self.apply_achievements(game, source, &list),
                        Err(e) => format!("{}: {e}", source.label()),
                    };
                    self.set_status(self.sync_note.clone());
                }
                Done::PageText { guide, text } => {
                    if self.reader_fetching == Some(guide) {
                        // for the reader: keep it beside the library
                        self.reader_fetching = None;
                        match text {
                            Ok(t) => {
                                match model::save_page(&self.pages_dir, guide, &t) {
                                    Ok(()) => self.set_status("Got the guide's text"),
                                    Err(e) => self.set_status_for(format!("Couldn't save the guide's text: {e}"), 6.0),
                                }
                                self.reader = None;
                            }
                            Err(e) => self.set_status_for(e, 8.0),
                        }
                    } else if let Some((gid, buf)) = &mut self.bulk {
                        // only if the paste window is still open for that guide
                        if *gid == guide {
                            self.bulk_fetching = false;
                            match text {
                                Ok(t) => {
                                    *buf = t;
                                    self.bulk_error = None;
                                }
                                Err(e) => self.bulk_error = Some(e),
                            }
                        }
                    }
                }
                Done::Link { game, guide, dead } => {
                    self.link_checks = self.link_checks.saturating_sub(1);
                    if let Some(g) = self.lib.game_mut(game).and_then(|g| g.guides.iter_mut().find(|x| x.id == guide)) {
                        if let Some(d) = dead {
                            g.dead = d;
                        }
                        if g.dead {
                            self.link_gone += 1;
                        }
                    }
                    if self.link_checks == 0 {
                        let msg = match self.link_gone {
                            0 => "Link check done: every page is still there".to_string(),
                            1 => "Link check done: 1 page is gone (it has an Archived copy button)".to_string(),
                            n => format!("Link check done: {n} pages are gone (they have Archived copy buttons)"),
                        };
                        self.set_status_for(msg, 6.0);
                    }
                }
            }
        }
        any
    }

    /// Queues a game's box art: its chosen file, the cached copy, or (online) a Wikipedia lookup.
    fn want_cover(&mut self, game: u64) {
        if self.covers.contains_key(&game) {
            return;
        }
        let Some(g) = self.lib.games.iter().find(|g| g.id == game) else { return };
        if g.cover == "none" {
            self.covers.insert(game, Fav::Missing);
            return;
        }
        let file = (!g.cover.is_empty()).then(|| PathBuf::from(&g.cover));
        self.covers.insert(game, Fav::Loading);
        self.online.send(Job::Cover { game, title: g.title.clone(), file, allow_net: self.online_on });
    }

    fn cover_of(&self, game: u64) -> Option<(egui::TextureId, Vec2)> {
        match self.covers.get(&game) {
            Some(Fav::Ready(t)) => Some((t.id(), t.size_vec2())),
            _ => None,
        }
    }

    /// Queues favicons for the hosts on screen. Offline, only cached icons are used.
    fn want_favicon(&mut self, host: &str) {
        if host.is_empty() || self.favicons.contains_key(host) {
            return;
        }
        let safe: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || ".-".contains(*c)).collect();
        let cached = online::cache_dir().join(format!("{safe}.png")).is_file();
        if self.online_on || cached {
            self.favicons.insert(host.to_string(), Fav::Loading);
            self.online.send(Job::Favicon { host: host.to_string() });
        } else {
            self.favicons.insert(host.to_string(), Fav::Missing);
        }
    }

    /// Saves library.json a moment after the last change.
    fn autosave(&mut self, check: bool) {
        let now = self.time();
        if check {
            let h = hash_lib(&self.lib);
            if h != self.last_hash {
                // The first change after a pause starts a new undo step (typing a note is one
                // step, not one per letter).
                if now - self.changed_at > 1.0 {
                    self.undo.push(std::mem::replace(&mut self.snapshot, self.lib.clone()));
                    if self.undo.len() > 100 {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                } else {
                    self.snapshot = self.lib.clone();
                }
                self.last_hash = h;
                self.changed_at = now;
            }
        }
        if self.last_hash != self.saved_hash && now - self.changed_at > 0.8 {
            self.flush();
        }
    }

    fn undo_redo(&mut self, undo: bool) {
        let (from, to) = if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(state) = from.pop() else {
            self.set_status(if undo { "Nothing to undo" } else { "Nothing to redo" });
            return;
        };
        to.push(std::mem::replace(&mut self.lib, state));
        self.snapshot = self.lib.clone();
        self.last_hash = hash_lib(&self.lib);
        self.changed_at = self.time();
        if self.selected.is_some_and(|id| !self.lib.games.iter().any(|g| g.id == id)) {
            self.selected = self.sorted_ids().first().copied();
        }
        self.set_status(if undo { "Undone" } else { "Redone" });
    }

    fn flush(&mut self) {
        if !self.persist {
            return;
        }
        let h = hash_lib(&self.lib);
        if h == self.saved_hash {
            return;
        }
        if let Err(e) = model::daily_backup(&self.lib_path) {
            self.set_status_for(format!("Couldn't make today's backup: {e}"), 6.0);
        }
        match model::save(&self.lib_path, &self.lib) {
            Ok(()) => {
                self.saved_hash = h;
                self.last_hash = h;
                self.file_mtime = mtime(&self.lib_path);
            }
            Err(e) => self.set_status_for(format!("Couldn't save: {e}"), 8.0),
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        let mut pasted = Vec::new();
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::Paste(s) = e {
                    pasted.push(s.clone());
                }
            }
        });
        // Ctrl+V anywhere (outside a text box) adds the link on the clipboard as a guide.
        if !typing && self.add_game.is_none() {
            for s in pasted {
                for link in links::extract_all(&s) {
                    self.add_guide(&link);
                }
            }
        }
        if !typing {
            let redo = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Z) || i.consume_key(egui::Modifiers::COMMAND, Key::Y));
            if redo {
                self.undo_redo(false);
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Z)) {
                self.undo_redo(true);
            }
        }
        if self.view != ViewMode::All && !typing && self.add_game.is_none() && self.bulk.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.view = ViewMode::All;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::F)) {
            self.search_all = Some(self.search_all.take().unwrap_or_default());
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::N)) {
            self.add_game = Some(NewGame { title: String::new(), platform: String::new(), status: Status::Playing, focus: true });
        }
        if !typing && ctx.input(|i| i.key_pressed(Key::Enter)) && self.add_game.is_none() {
            self.continue_game();
        }
    }

    // -----------------------------------------------------------------------------------------
    // Header

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(150.0, 52.0), Sense::hover());
            let p = ui.painter();
            p.text(r.left_center() + Vec2::new(3.0, 3.0), Align2::LEFT_CENTER, "JAGT", bold(48.0), MAGENTA);
            glow_text(p, r.left_center(), Align2::LEFT_CENTER, "JAGT", bold(48.0), MAGENTA);
            p.text(r.left_center(), Align2::LEFT_CENTER, "JAGT", bold(48.0), YELLOW);
            ui.vertical(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new("Just Another").color(TEAL));
                ui.label(RichText::new("Guide Tracker").color(TEAL));
            });
            ui.add_space(18.0);

            let filters: [Option<Status>; 6] = [None, Some(Status::Playing), Some(Status::Backlog), Some(Status::Paused), Some(Status::Completed), Some(Status::Dropped)];
            for f in filters {
                let count = self.lib.games.iter().filter(|g| f.is_none_or(|s| g.status == s)).count();
                if desktop_icon(ui, f, self.filter == f, count).clicked() {
                    self.filter = f;
                }
            }
            ui.add_space(10.0);
            if explorer_icon(ui, self.explorer_open).clicked() {
                self.explorer_open = !self.explorer_open;
                self.trophies_open = false;
                self.missables_open = false;
            }
            if trophies_icon(ui, self.trophies_open).clicked() {
                self.trophies_open = !self.trophies_open;
                self.explorer_open = false;
                self.missables_open = false;
            }
            if missables_icon(ui, self.missables_open).clicked() {
                self.missables_open = !self.missables_open;
                self.explorer_open = false;
                self.trophies_open = false;
            }
            ui.add_space(14.0);
            // one compact dropdown, so the header fits on narrower windows
            ui.label(RichText::new("Show").color(TEAL));
            let shown = if self.explorer_open { "Explorer" } else { self.view.label() };
            egui::ComboBox::from_id_salt("show-view")
                .selected_text(RichText::new(shown).color(YELLOW))
                .width(96.0)
                .show_ui(ui, |ui| {
                    for v in ViewMode::ALL {
                        let tip = match v {
                            ViewMode::All => "All windows (the usual layout)".to_string(),
                            ViewMode::Notes => "Only Where I Left Off and the Session Log".to_string(),
                            other => format!("Only the {} window", other.label()),
                        };
                        if ui.selectable_label(self.view == v && !self.explorer_open, v.label()).on_hover_text(tip).clicked() {
                            self.view = v;
                            self.explorer_open = false;
                            self.trophies_open = false;
                            self.missables_open = false;
                        }
                    }
                })
                .response
                .on_hover_text("Which windows get the space: all of them, or just one (Esc goes back to all)");

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                let online = ui
                    .selectable_label(self.online_on, if self.online_on { "Online: on" } else { "Online: off" })
                    .on_hover_text("Look up guide titles and site icons online.\nOff: nothing leaves your PC (cached icons still show).");
                if online.clicked() {
                    self.online_on = !self.online_on;
                    if self.online_on {
                        self.favicons.retain(|_, f| !matches!(f, Fav::Missing));
                        self.covers.retain(|_, f| !matches!(f, Fav::Missing));
                    }
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo")).on_hover_text("Ctrl+Y").clicked() {
                    self.undo_redo(false);
                }
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
                    self.undo_redo(true);
                }
                if ui.add(egui::Button::new("Settings").selected(self.show_settings)).clicked() {
                    self.show_settings = !self.show_settings;
                }
                let has_game = self.selected.is_some();
                if ui
                    .add_enabled(has_game, egui::Button::new("Add folder"))
                    .on_hover_text("Add every guide file in a folder to this game")
                    .on_disabled_hover_text("Pick a game first")
                    .clicked()
                {
                    self.pick_folder();
                }
                if ui
                    .add_enabled(has_game, egui::Button::new("Add files"))
                    .on_hover_text("Add guide files from your PC to this game: PDFs, saved pages, spreadsheets, maps...\nYou can also drop files or folders onto the window.")
                    .on_disabled_hover_text("Pick a game first")
                    .clicked()
                {
                    self.pick_files();
                }
                if ui.button("Find").on_hover_text("Search every game, guide and checklist (Ctrl+F)").clicked() {
                    self.search_all = Some(String::new());
                }
                if ui.button("+ Add game").on_hover_text("Ctrl+N").clicked() {
                    self.add_game = Some(NewGame { title: String::new(), platform: String::new(), status: Status::Playing, focus: true });
                }
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Games list (left)

    fn games_view(&mut self, ui: &mut Ui) {
        let (_, add, _) = input_row(ui, &mut self.search, "Search games…", "search-games", "+ Add");
        if add {
            self.add_game = Some(NewGame { title: String::new(), platform: String::new(), status: self.filter.unwrap_or(Status::Playing), focus: true });
        }
        ui.horizontal(|ui| {
            if ui.selectable_label(self.folder_view, "Folders").on_hover_text("Group games into PC / Console / Mobile / Linux folders").clicked() {
                self.folder_view = true;
            }
            if ui.selectable_label(!self.folder_view, "List").on_hover_text("One flat list").clicked() {
                self.folder_view = false;
            }
        });
        // your own tags, as filters
        let mut tags: Vec<String> = Vec::new();
        for g in &self.lib.games {
            for t in &g.tags {
                if !tags.contains(t) {
                    tags.push(t.clone());
                }
            }
        }
        if !tags.is_empty() {
            tags.sort_by_key(|t| t.to_lowercase());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for t in &tags {
                    let on = self.tag_filter.as_ref() == Some(t);
                    if ui.selectable_label(on, RichText::new(format!("#{t}")).small()).on_hover_text("Only games with this tag (click again for all)").clicked() {
                        self.tag_filter = if on { None } else { Some(t.clone()) };
                    }
                }
            });
        }
        let ids = self.sorted_ids();
        if ids.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                if self.lib.games.is_empty() {
                    ui.label(RichText::new("No games yet").color(TEAL));
                    ui.label(RichText::new("Add one with + Add").small().color(DIM));
                } else {
                    ui.label(RichText::new("Nothing matches").color(TEAL));
                    if self.filter.is_some() && ui.button("Show all games").clicked() {
                        self.filter = None;
                    }
                }
            });
            ui.allocate_space(ui.available_size());
            return;
        }

        let width = ui.available_width();
        let mut acts: Vec<GameAct> = Vec::new();
        let mut toggle: Option<String> = None;
        let mut nav: Option<Vec<String>> = None;
        let explorer = self.explorer_open;
        let searching = !self.search.trim().is_empty();
        let games: Vec<&Game> = ids.iter().filter_map(|id| self.lib.games.iter().find(|g| g.id == *id)).collect();
        let selected = self.selected;
        let closed = &self.closed_folders;
        let is_open = |key: &str| searching || !closed.iter().any(|k| k == key);

        egui::ScrollArea::vertical().id_salt("games").auto_shrink([false, false]).show(ui, |ui| {
            if !self.folder_view {
                for (n, g) in games.iter().enumerate() {
                    game_row(ui, g, selected == Some(g.id), n, 0.0, width, &mut acts);
                }
                return;
            }
            let mut n = 0;
            for f in Folder::ALL {
                let in_folder: Vec<&&Game> = games.iter().filter(|g| g.folder() == f).collect();
                if in_folder.is_empty() {
                    continue;
                }
                let key = f.label().to_string();
                let open = is_open(&key);
                if folder_row(ui, f.label(), in_folder.len(), open, 0, width).clicked() {
                    if explorer {
                        nav = Some(vec![key.clone()]);
                    } else {
                        toggle = Some(key.clone());
                    }
                }
                if !open {
                    continue;
                }
                // a subfolder per platform, when the folder holds more than one
                let mut platforms: Vec<String> = Vec::new();
                for g in &in_folder {
                    let name = platform_name(&g.platform);
                    if !platforms.iter().any(|p| p.eq_ignore_ascii_case(&name)) {
                        platforms.push(name);
                    }
                }
                platforms.sort_by_key(|p| p.to_lowercase());
                if platforms.len() < 2 {
                    for g in &in_folder {
                        game_row(ui, g, selected == Some(g.id), n, 1.0, width, &mut acts);
                        n += 1;
                    }
                    continue;
                }
                for pl in &platforms {
                    let here: Vec<&&&Game> = in_folder.iter().filter(|g| platform_name(&g.platform).eq_ignore_ascii_case(pl)).collect();
                    let sub = format!("{key}/{}", pl.to_lowercase());
                    let open = is_open(&sub);
                    if folder_row(ui, pl, here.len(), open, 1, width).clicked() {
                        if explorer {
                            nav = Some(vec![key.clone(), pl.clone()]);
                        } else {
                            toggle = Some(sub.clone());
                        }
                    }
                    if open {
                        for g in here {
                            game_row(ui, g, selected == Some(g.id), n, 2.0, width, &mut acts);
                            n += 1;
                        }
                    }
                }
            }
        });

        if let Some(key) = toggle {
            if let Some(i) = self.closed_folders.iter().position(|k| *k == key) {
                self.closed_folders.remove(i);
            } else {
                self.closed_folders.push(key);
            }
        }
        if let Some(path) = nav {
            // the tree is the explorer's navigation pane: open the folder there, and here
            self.closed_folders.retain(|k| *k != path[0] && !path.get(1).is_some_and(|pl| *k == format!("{}/{}", path[0], pl.to_lowercase())));
            self.ex_go(path);
        }
        self.apply_game_acts(acts);
    }

    // -----------------------------------------------------------------------------------------
    // Selected game (centre)

    fn game_header(&mut self, ui: &mut Ui, gid: u64) {
        let now = self.time();
        let mut delete = false;
        let mut sync = false;
        self.want_cover(gid);
        let art = self.cover_of(gid);
        let mut cover_change: Option<String> = None;
        let mut refetch_cover = false;
        let Some(g) = self.lib.game_mut(gid) else { return };
        ui.horizontal(|ui| {
            // box art thumbnail, with the Cover menu on it
            let (r, resp) = ui.allocate_exact_size(Vec2::new(30.0, 40.0), Sense::click());
            icons::game_case_art(ui.painter(), r, platform_color(&g.platform), art, None, g.status.color());
            let resp = resp.on_hover_text("Cover (right-click for options)");
            resp.context_menu(|ui| {
                ui.label(RichText::new("Cover").color(TEAL));
                if ui.selectable_label(g.cover.is_empty(), "Automatic (Wikipedia)").clicked() {
                    cover_change = Some(String::new());
                    ui.close_menu();
                }
                if ui.button("Choose image\u{2026}").clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file() {
                        cover_change = Some(f.display().to_string());
                    }
                    ui.close_menu();
                }
                if ui.selectable_label(g.cover == "none", "No cover").clicked() {
                    cover_change = Some("none".into());
                    ui.close_menu();
                }
                if g.cover.is_empty() && ui.button("Look it up again").clicked() {
                    refetch_cover = true;
                    ui.close_menu();
                }
            });
            ui.add(egui::TextEdit::singleline(&mut g.title).font(bold(24.0)).desired_width(f32::INFINITY).frame(false).hint_text("Game title"));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Platform").color(TEAL));
            ui.add(egui::TextEdit::singleline(&mut g.platform).hint_text("PS5, Switch, PC…").desired_width(130.0));
            ui.add_space(6.0);
            ui.label(RichText::new("Folder").color(TEAL));
            let auto = Folder::of_platform(&g.platform);
            let shown = match g.folder {
                Some(f) => f.label().to_string(),
                None => format!("Auto ({})", auto.label()),
            };
            egui::ComboBox::from_id_salt(("folder", gid)).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut g.folder, None, format!("Automatic ({})", auto.label()));
                for f in Folder::ALL {
                    ui.selectable_value(&mut g.folder, Some(f), f.label());
                }
            })
            .response
            .on_hover_text("Which folder this game sits in on the left. Automatic goes by the platform.");
            ui.add_space(6.0);
            ui.label(RichText::new("Status").color(TEAL));
            egui::ComboBox::from_id_salt(("status", gid)).selected_text(RichText::new(g.status.label()).color(g.status.color())).show_ui(ui, |ui| {
                for s in Status::ALL {
                    ui.selectable_value(&mut g.status, s, RichText::new(s.label()).color(s.color()));
                }
            });
            if g.has_deadlines() || !g.game_date.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("In-game date").color(TEAL));
                let r = ui
                    .add(egui::TextEdit::singleline(&mut g.game_date).id_salt(("date", gid)).hint_text("M/D").desired_width(56.0))
                    .on_hover_text("Today's date in the game, as month/day (e.g. 4/2).\nItems ending within 2 weeks are highlighted; ones past their date count as missed.");
                if r.changed() {
                    let today = g.today();
                    g.guides.iter_mut().for_each(|x| x.follow_checklist(today));
                }
                if !g.game_date.trim().is_empty() && g.today().is_none() {
                    ui.label(RichText::new("use M/D").small().color(MAGENTA));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                delete = armed_button(ui, ("del-game", gid), "Delete game", now);
                if ui.button("Sync achievements\u{2026}").on_hover_text("Tick earned achievements from Steam or Xbox").clicked() {
                    sync = true;
                }
            });
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let pr = g.progress();
            let (r, _) = ui.allocate_exact_size(Vec2::new(160.0, 16.0), Sense::hover());
            let bar = Rect::from_center_size(r.center(), Vec2::new(r.width(), 8.0));
            ui.painter().rect_filled(bar, 2.0, INK);
            ui.painter().rect_stroke(bar.expand(1.0), 2.0, Stroke::new(1.0f32, INDIGO), StrokeKind::Outside);
            ui.painter().rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.left() + bar.width() * pr, bar.bottom())), 2.0, MAGENTA);
            let mut info = vec![format!("{:.0}% across guides", pr * 100.0)];
            let at = model::now();
            if g.played(at) >= 60 {
                info.push(format!("played {}", model::fmt_duration(g.played(at))));
            }
            if let Some(left) = g.estimate_left(at) {
                info.push(format!("~{} left at your pace", model::fmt_duration(left)));
            }
            info.push(format!("added {}", model::date(g.added)));
            info.push(match g.last_played {
                Some(t) => format!("last played {}", model::ago(t)),
                None => "not played yet".into(),
            });
            ui.label(RichText::new(info.join(" · ")).small().color(DIM));
        });
        // ---- play timer, run, tags
        let mut timer: Option<bool> = None;
        let mut new_run = false;
        let mut switch_run: Option<usize> = None;
        let mut delete_run: Option<usize> = None;
        let all_tags: Vec<String> = {
            let mut v: Vec<String> = Vec::new();
            for t in self.lib.games.iter().flat_map(|x| &x.tags) {
                if !v.contains(t) {
                    v.push(t.clone());
                }
            }
            v
        };
        let new_tag = &mut self.new_tag;
        let Some(g) = self.lib.game_mut(gid) else { return };
        ui.horizontal_wrapped(|ui| {
            let at = model::now();
            match g.session_start {
                Some(s) => {
                    let label = format!("\u{25a0} Stop  {}", model::fmt_clock((at - s).max(0) as u64));
                    if ui.add(egui::Button::new(RichText::new(label).color(MAGENTA)).selected(true)).on_hover_text("Stop the play timer (the session goes in the log)").clicked() {
                        timer = Some(false);
                    }
                }
                None => {
                    if ui.button("\u{25b6} Start playing").on_hover_text("Start the play timer (Continue starts it too)").clicked() {
                        timer = Some(true);
                    }
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Run").color(TEAL));
            ui.menu_button(RichText::new(g.run_label()).color(YELLOW), |ui| {
                ui.label(RichText::new("Each run keeps its own ticks (New Game+, a second route...)").small().color(DIM));
                if ui.button("Start a new run\u{2026}").clicked() {
                    new_run = true;
                    ui.close_menu();
                }
                ui.label(RichText::new("Rename this run").small().color(TEAL));
                let hint = g.run_label();
                ui.add(egui::TextEdit::singleline(&mut g.run_name).hint_text(hint).desired_width(180.0));
                if !g.runs.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("Earlier runs").small().color(TEAL));
                    let rows: Vec<(String, String)> = g
                        .runs
                        .iter()
                        .map(|r| {
                            let (d, n) = r.counts(g);
                            (r.name.clone(), format!("{d}/{n} ticked \u{b7} {} \u{b7} {}", model::fmt_duration(r.play_secs), model::date(r.ended)))
                        })
                        .collect();
                    for (i, (name, info)) in rows.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).color(WHITE));
                            ui.label(RichText::new(info).small().color(DIM));
                            if ui.small_button("Switch to it").on_hover_text("Bring back this run's ticks (the current run is kept too)").clicked() {
                                switch_run = Some(i);
                                ui.close_menu();
                            }
                            if ui.small_button("Delete").clicked() {
                                delete_run = Some(i);
                                ui.close_menu();
                            }
                        });
                    }
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Tags").color(TEAL));
            let mut remove = None;
            for (i, t) in g.tags.iter().enumerate() {
                if ui.small_button(format!("#{t}  \u{d7}")).on_hover_text("Remove this tag").clicked() {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                g.tags.remove(i);
            }
            let r = ui.add(egui::TextEdit::singleline(new_tag).id_salt(("new-tag", gid)).hint_text("+ tag").desired_width(80.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let t = new_tag.trim().trim_start_matches('#').trim().to_string();
                if !t.is_empty() && !g.tags.contains(&t) {
                    g.tags.push(t);
                }
                new_tag.clear();
            }
            let others: Vec<&String> = all_tags.iter().filter(|t| !g.tags.contains(t)).collect();
            if !others.is_empty() {
                ui.menu_button("\u{25be}", |ui| {
                    for t in others {
                        if ui.button(format!("#{t}")).clicked() {
                            g.tags.push(t.clone());
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Add a tag you've used before");
            }
        });
        if let Some(i) = delete_run {
            g.runs.remove(i);
        }
        let mut switched = None;
        if let Some(i) = switch_run {
            g.switch_run(i, model::now());
            switched = Some(format!("Switched to {}", g.run_label()));
        }
        if new_run {
            // keep trophy lists ticked by default
            let keep: HashSet<u64> = g.guides.iter().filter(|x| x.kind == Kind::Trophy).map(|x| x.id).collect();
            self.run_window = Some((gid, format!("Run {} (NG+)", g.runs.len() + 2), keep, true));
        }
        if let Some(m) = switched {
            self.set_status(m);
        }
        match timer {
            Some(true) => self.start_playing(gid),
            Some(false) => self.stop_playing(gid),
            None => {}
        }
        let Some(g) = self.lib.game_mut(gid) else { return };
        if let Some(c) = cover_change {
            g.cover = c;
            self.covers.remove(&gid);
        }
        if refetch_cover {
            let _ = std::fs::remove_file(online::cover_cache(gid));
            self.covers.remove(&gid);
        }
        if sync {
            self.sync_window = Some(gid);
            self.sync_note.clear();
        }
        if delete {
            let title = g.title.clone();
            self.lib.games.retain(|g| g.id != gid);
            self.selected = self.sorted_ids().first().copied();
            self.set_status(format!("Deleted {title}"));
        }
    }

    fn guides_view(&mut self, ui: &mut Ui, gid: u64, max_h: f32) {
        let (_, add, enter) = input_row(ui, &mut self.new_url, "Paste a guide link: website, spreadsheet, YouTube…", "new-guide", "+ Add guide");
        if (add || enter) && !self.new_url.trim().is_empty() {
            let text = std::mem::take(&mut self.new_url);
            let found = links::extract_all(&text);
            let added = found.iter().filter(|l| self.add_guide(l)).count();
            if found.is_empty() {
                self.set_status("That doesn't look like a link");
                self.new_url = text;
            } else if found.len() > 1 {
                self.set_status(format!("Added {added} of {} links", found.len()));
            }
        }
        ui.add_space(4.0);

        let Some(game) = self.lib.games.iter().find(|g| g.id == gid) else { return };
        if game.guides.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No guides yet").color(TEAL));
                ui.label(RichText::new("Paste a link above, or press Ctrl+V anywhere with a link copied.").small().color(DIM));
                ui.label(RichText::new("YouTube remembers your time, Google Sheets your row, and web pages a heading to jump back to.").small().color(DIM));
            });
            ui.allocate_space(ui.available_size());
            return;
        }
        let hosts: Vec<String> = game.guides.iter().map(|g| links::host(&g.url)).collect();
        for h in &hosts {
            self.want_favicon(h);
        }

        let mut acts: Vec<(u64, CardAct)> = Vec::new();
        let mut progressed = false;
        let App { lib, favicons, expanded, time_bufs, active_list, .. } = self;
        let Some(game) = lib.game_mut(gid) else { return };
        let last_used = game.continue_guide().map(|g| g.id);
        let count = game.guides.len();
        let active_list = *active_list;
        egui::ScrollArea::vertical().id_salt(("guides", gid)).auto_shrink([false, true]).max_height(max_h).show(ui, |ui| {
            for (i, g) in game.guides.iter_mut().enumerate() {
                let fav = match favicons.get(&hosts[i]) {
                    Some(Fav::Ready(t)) => Some(t),
                    _ => None,
                };
                let mut card = CardCtx {
                    fav,
                    expanded: expanded.contains(&g.id),
                    last_used: last_used == Some(g.id),
                    first: i == 0,
                    last: i + 1 == count,
                    time_buf: time_bufs.entry(g.id).or_insert_with(|| links::fmt_hms(g.video_secs)),
                    active: active_list == Some(g.id),
                    progressed: false,
                    toggle: false,
                };
                let id = g.id;
                for a in guide_card(ui, g, &mut card) {
                    acts.push((id, a));
                }
                if card.toggle && !expanded.remove(&id) {
                    expanded.insert(id);
                }
                progressed |= card.progressed;
                ui.add_space(6.0);
            }
        });
        if progressed {
            game.touch();
        }

        for (id, a) in acts {
            let Some(game) = self.lib.game_mut(gid) else { break };
            let Some(i) = game.guides.iter().position(|g| g.id == id) else { continue };
            match a {
                CardAct::Open => self.open_guide(gid, id),
                CardAct::Up if i > 0 => game.guides.swap(i, i - 1),
                CardAct::Down if i + 1 < game.guides.len() => game.guides.swap(i, i + 1),
                CardAct::Remove => {
                    let g = game.guides.remove(i);
                    if game.last_guide == Some(id) {
                        game.last_guide = None;
                    }
                    self.set_status(format!("Removed {}", g.title));
                }
                CardAct::Refetch => {
                    let url = game.guides[i].url.clone();
                    game.guides[i].auto_title = true;
                    if self.online_on {
                        self.online.send(Job::Title { game: gid, guide: id, url });
                        self.set_status("Looking up the title…");
                    } else {
                        self.set_status("Turn Online on to look up titles");
                    }
                }
                CardAct::BulkSteps => {
                    self.bulk = Some((id, String::new()));
                    self.active_list = Some(id);
                }
                CardAct::ShowChecklist => self.active_list = Some(id),
                CardAct::Archive => {
                    let url = online::archive_url(&game.guides[i].url);
                    if let Err(e) = open::that_detached(&url) {
                        self.set_status_for(format!("Couldn't open the browser: {e}"), 6.0);
                    }
                }
                CardAct::CheckLink => {
                    if self.online_on {
                        let url = game.guides[i].url.clone();
                        self.online.send(Job::CheckLink { game: gid, guide: id, url });
                        self.link_checks += 1;
                        self.link_gone = 0;
                        self.set_status("Checking the link\u{2026}");
                    } else {
                        self.set_status("Turn Online on to check links");
                    }
                }
                CardAct::ShowInFolder => {
                    if let Some(path) = links::local_path(&game.guides[i].url) {
                        if let Err(e) = show_in_folder(&path) {
                            self.set_status_for(format!("Couldn't open the folder: {e}"), 6.0);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Checklist (centre, below the guides)

    fn checklist_view(&mut self, ui: &mut Ui, gid: u64) {
        let Some(game) = self.lib.games.iter().find(|g| g.id == gid) else { return };
        let today = game.today();
        let active = self
            .active_list
            .filter(|id| game.guides.iter().any(|g| g.id == *id))
            .or_else(|| game.guides.iter().find(|g| !g.steps.is_empty()).map(|g| g.id))
            .or_else(|| game.guides.first().map(|g| g.id));
        if active != self.active_list {
            self.list_section = None;
        }
        self.active_list = active;
        let Some(aid) = active else { return };
        let choices: Vec<(u64, String, usize)> = game.guides.iter().map(|g| (g.id, g.title.clone(), g.steps.len())).collect();
        let has_steps = choices.iter().any(|c| c.0 == aid && c.2 > 0);
        let guard = game.spoiler_guard;
        let mut toggle_guard = false;

        // ---- toolbar: which guide, view switch and actions
        let mut paste = false;
        let mut untick = false;
        let mut tick_list = false;
        let mut clear = false;
        ui.horizontal(|ui| {
            if choices.len() > 1 {
                let cur = choices.iter().find(|c| c.0 == aid).map(|c| c.1.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt(("list-guide", gid))
                    .width((ui.available_width() - 330.0).clamp(160.0, 420.0))
                    .selected_text(RichText::new(cur).font(bold(16.0)))
                    .show_ui(ui, |ui| {
                        for (id, title, n) in &choices {
                            let label = if *n > 0 { format!("{title}  ({n})") } else { title.clone() };
                            if ui.selectable_label(*id == aid, label).clicked() {
                                self.active_list = Some(*id);
                                self.list_section = None;
                            }
                        }
                    });
            } else if let Some(c) = choices.first() {
                ui.label(RichText::new(&c.1).font(bold(16.0)).color(WHITE));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("…", |ui| {
                    if ui.button("Tick from a list\u{2026}").on_hover_text("Paste names (e.g. a trophy list from PSNProfiles) and tick the matching items").clicked() {
                        tick_list = true;
                        ui.close_menu();
                    }
                    let mut g = guard;
                    if ui
                        .checkbox(&mut g, "Spoiler guard")
                        .on_hover_text("Cover the names and details of items after your next one, until you click them (this game)")
                        .changed()
                    {
                        toggle_guard = true;
                        ui.close_menu();
                    }
                    if ui.button("Untick everything").clicked() {
                        untick = true;
                        ui.close_menu();
                    }
                    if ui.button(RichText::new("Delete the whole checklist").color(MAGENTA)).clicked() {
                        clear = true;
                        ui.close_menu();
                    }
                });
                if ui.button("Paste checklist…").clicked() {
                    paste = true;
                }
                if has_steps {
                    if ui
                        .add(egui::Button::new("Guide text").selected(self.reader_open))
                        .on_hover_text("Read the guide here, beside the checklist, following your place")
                        .clicked()
                    {
                        self.reader_open = !self.reader_open;
                    }
                    let focused = self.view == ViewMode::Checklist;
                    let focus_label = if focused { "Exit focus" } else { "Focus" };
                    if ui.add(egui::Button::new(focus_label).selected(focused)).on_hover_text("Give the checklist the whole window (Esc to leave)").clicked() {
                        self.view = if focused { ViewMode::All } else { ViewMode::Checklist };
                    }
                    if ui
                        .add(egui::Button::new("Compact").selected(self.compact_rows))
                        .on_hover_text("One line per item: click the arrow (or hover) for its details")
                        .clicked()
                    {
                        self.compact_rows = !self.compact_rows;
                    }
                }
                if has_steps {
                    if ui.add(egui::Button::new("Full list").selected(!self.left_view)).clicked() {
                        self.left_view = false;
                    }
                    if ui.add(egui::Button::new("What's left").selected(self.left_view)).on_hover_text("What's still to do, area by area").clicked() {
                        self.left_view = true;
                    }
                }
            });
        });

        if tick_list {
            self.tick_list = Some((aid, String::new(), TickMode::Named));
        }
        let compact = self.compact_rows;
        let ctx = ui.ctx().clone();
        let App { lib, list_kind, list_scope, hide_done, list_search, list_section, left_view, collapsed, scroll_to_next, step_bufs, row_heights, open_rows, list_cursor, revealed, thumbs, scroll_to_item, note_edit, .. } = self;
        let Some(game) = lib.game_mut(gid) else { return };
        if toggle_guard {
            game.spoiler_guard = !game.spoiler_guard;
        }
        let guard = game.spoiler_guard;
        let Some(guide) = game.guides.iter_mut().find(|g| g.id == aid) else { return };
        if untick {
            guide.steps.iter_mut().for_each(|s| {
                s.done = false;
                s.missed = false;
            });
            guide.follow_checklist(today);
        }
        if clear {
            guide.steps.clear();
        }

        if guide.steps.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No checklist for this guide yet").font(bold(20.0)).color(TEAL));
                ui.add_space(6.0);
                ui.label(RichText::new("Fetch it from the guide page, or copy the guide's text and paste it in: chests, objectives, achievements, quests and their How / Reward lines become ticks.").color(DIM));
                ui.label(RichText::new("Or add items one at a time below.").color(DIM));
                ui.add_space(10.0);
                if ui.button(RichText::new("Paste or fetch checklist…").font(bold(18.0))).clicked() {
                    paste = true;
                }
            });
            ui.add_space(16.0);
            add_item_row(ui, guide, step_bufs.entry(aid).or_default());
            ui.allocate_space(ui.available_size());
            if paste {
                self.bulk = Some((aid, String::new()));
            }
            return;
        }

        // ---- overall progress
        let states: Vec<StepState> = guide.steps.iter().map(|s| s.state(today)).collect();
        let total = guide.steps.len();
        let done = states.iter().filter(|s| **s == StepState::Done).count();
        let missed = states.iter().filter(|s| **s == StepState::Missed).count();
        let due = states.iter().filter(|s| **s == StepState::DueSoon).count();
        let counted = (total - missed).max(1);
        ui.horizontal(|ui| {
            let frac = done as f32 / counted as f32;
            let (r, _) = ui.allocate_exact_size(Vec2::new((ui.available_width() - 260.0).max(80.0), 20.0), Sense::hover());
            let bar = Rect::from_center_size(r.center(), Vec2::new(r.width(), 10.0));
            rail(ui, bar, frac, false, ("list-rail", aid));
            let col = if done == counted { GREEN } else { YELLOW };
            ui.label(RichText::new(format!("{done} / {counted} · {:.0}%", frac * 100.0)).font(bold(16.0)).color(col));
            if due > 0 {
                ui.label(RichText::new(format!("{due} due soon")).color(MAGENTA));
            }
            if missed > 0 {
                ui.label(RichText::new(format!("{missed} missed")).color(DIM));
            }
        });

        let next = guide.next_index(today);
        let mut acts: Vec<(usize, RowAct)> = Vec::new();
        // spoiler guard: open items after the next one are covered until clicked
        let veiled = |i: usize| guard && next.is_some_and(|n| i > n) && matches!(states[i], StepState::Open | StepState::DueSoon) && !revealed.contains(&(aid, i));

        if *left_view {
            // ---- What's left: area by area
            let mut areas: Vec<(String, Vec<usize>)> = Vec::new();
            for (i, s) in guide.steps.iter().enumerate() {
                if !matches!(states[i], StepState::Open | StepState::DueSoon) {
                    continue;
                }
                match areas.iter_mut().find(|a| a.0 == s.section) {
                    Some(a) => a.1.push(i),
                    None => areas.push((s.section.clone(), vec![i])),
                }
            }
            let left: usize = areas.iter().map(|a| a.1.len()).sum();
            let mut view_area = None;
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt(("left", aid)).auto_shrink([false, false]).show(ui, |ui| {
                if left == 0 {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("Nothing left. All done!").font(bold(20.0)).color(GREEN)));
                    return;
                }
                ui.label(RichText::new(format!("{left} left: {}", kind_counts(&guide.steps, areas.iter().flat_map(|a| a.1.iter().copied())))).font(bold(18.0)).color(YELLOW));
                ui.add_space(6.0);
                for (sec, idx) in &areas {
                    egui::Frame::new().fill(CARD_BG).stroke(Stroke::new(1.0f32, INDIGO)).corner_radius(3).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let name = if sec.is_empty() { "Other" } else { sec.as_str() };
                            ui.label(RichText::new(name).font(bold(18.0)).color(YELLOW));
                            ui.label(RichText::new(kind_counts(&guide.steps, idx.iter().copied())).color(TEAL));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button("Show these").on_hover_text("Open the list at this area, finished items hidden").clicked() {
                                    view_area = Some(sec.clone());
                                }
                            });
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for &i in idx {
                                let s = &guide.steps[i];
                                let col = if states[i] == StepState::DueSoon { MAGENTA } else if Some(i) == next { MAGENTA } else { WHITE };
                                let mut text = s.text.clone();
                                if states[i] == StepState::DueSoon {
                                    text = format!("{text} (ends {})", s.deadline);
                                }
                                if veiled(i) {
                                    let chip = egui::Frame::new().fill(INK).stroke(Stroke::new(1.0f32, INDIGO)).corner_radius(2).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
                                        ui.label(RichText::new("\u{2592}\u{2592}\u{2592}\u{2592}\u{2592}").color(DIM));
                                    });
                                    if chip.response.interact(Sense::click()).on_hover_text("Hidden by the spoiler guard: click to show").clicked() {
                                        acts.push((i, RowAct::Reveal));
                                    }
                                    continue;
                                }
                                let chip = egui::Frame::new().fill(INK).stroke(Stroke::new(1.0f32, if Some(i) == next { MAGENTA } else { INDIGO })).corner_radius(2).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
                                    let (ir, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                    icons::step_icon(ui.painter(), ir, s.kind, false);
                                    ui.label(RichText::new(text).color(col));
                                });
                                let r = chip.response.interact(Sense::click()).on_hover_text("Click to tick off");
                                if r.clicked() {
                                    acts.push((i, RowAct::Toggle));
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
            });
            if let Some(sec) = view_area {
                *left_view = false;
                *hide_done = true;
                collapsed.remove(&(aid, sec.clone()));
                *list_section = Some(sec);
            }
        } else {
            // ---- filters
            let kinds = StepKind::ALL.iter().filter(|k| guide.steps.iter().any(|s| s.kind == **k)).count();
            if kinds < 2 {
                *list_kind = None;
            }
            let any_optional = guide.steps.iter().any(|s| s.optional);
            if !any_optional {
                *list_scope = None;
            }
            ui.horizontal_wrapped(|ui| {
                if kinds > 1 && ui.selectable_label(list_kind.is_none(), "All types").clicked() {
                    *list_kind = None;
                }
                for k in StepKind::ALL.into_iter().filter(|_| kinds > 1) {
                    let n = guide.steps.iter().filter(|s| s.kind == k).count();
                    if n == 0 {
                        continue;
                    }
                    let d = guide.steps.iter().filter(|s| s.kind == k && s.done).count();
                    if ui.selectable_label(*list_kind == Some(k), format!("{} {d}/{n}", k.plural())).clicked() {
                        *list_kind = if *list_kind == Some(k) { None } else { Some(k) };
                    }
                }
                if any_optional {
                    if kinds > 1 {
                        ui.add_space(6.0);
                    }
                    for (scope, label) in [(Some(false), "Story"), (Some(true), "Optional")] {
                        let n = guide.steps.iter().filter(|s| Some(s.optional) == scope).count();
                        if ui.selectable_label(*list_scope == scope, format!("{label} {n}")).clicked() {
                            *list_scope = if *list_scope == scope { None } else { scope };
                        }
                    }
                }
                ui.add_space(8.0);
                ui.checkbox(hide_done, "Hide done");
                ui.add(egui::TextEdit::singleline(list_search).hint_text("Search…").desired_width(140.0));
                if ui.button("Go to next").on_hover_text("Scroll to the next thing to do").clicked() {
                    *scroll_to_next = true;
                    if let Some(n) = next {
                        collapsed.remove(&(aid, guide.steps[n].section.clone()));
                        if list_section.as_ref().is_some_and(|s| *s != guide.steps[n].section) {
                            *list_section = None;
                        }
                    }
                }
                if let Some(sec) = list_section.clone() {
                    if ui.button(RichText::new(format!("Area: {sec}  x")).color(TEAL)).on_hover_text("Show every area again").clicked() {
                        *list_section = None;
                    }
                }
            });
            ui.add_space(4.0);

            // ---- the list
            let q = list_search.to_lowercase();
            let (kind_filter, scope, hide, area_filter) = (*list_kind, *list_scope, *hide_done, list_section.clone());
            let visible = |i: usize, s: &Step| {
                kind_filter.is_none_or(|k| s.kind == k)
                    && scope.is_none_or(|o| s.optional == o)
                    && (!hide || matches!(states[i], StepState::Open | StepState::DueSoon))
                    && area_filter.as_ref().is_none_or(|a| *a == s.section)
                    && (q.is_empty() || [&s.text, &s.detail, &s.how, &s.reward, &s.section].iter().any(|t| t.to_lowercase().contains(&q)))
            };
            // per-area counts (respecting the type / story filters)
            let mut areas: Vec<(String, usize, usize)> = Vec::new();
            for (i, s) in guide.steps.iter().enumerate() {
                if !kind_filter.is_none_or(|k| s.kind == k) || !scope.is_none_or(|o| s.optional == o) || states[i] == StepState::Missed {
                    continue;
                }
                let d = (states[i] == StepState::Done) as usize;
                match areas.iter_mut().find(|a| a.0 == s.section) {
                    Some(a) => {
                        a.1 += d;
                        a.2 += 1;
                    }
                    None => areas.push((s.section.clone(), d, 1)),
                }
            }
            let web = guide.kind != Kind::Video && guide.kind != Kind::Sheet && !links::is_local(&guide.url);
            let shown = guide.steps.iter().enumerate().filter(|(i, s)| visible(*i, s)).count();
            let scroll_next = std::mem::take(scroll_to_next);
            let mut toggle_section = None;

            // keyboard: a highlight that moves through the items on show
            let navigable: Vec<usize> = (0..guide.steps.len())
                .filter(|&i| {
                    let s = &guide.steps[i];
                    visible(i, s) && !(!s.section.is_empty() && collapsed.contains(&(aid, s.section.clone())))
                })
                .collect();
            let typing = ui.ctx().memory(|m| m.focused().is_some());
            let mut cursor_moved = false;
            if list_cursor.is_some_and(|(g, _)| g != aid) {
                *list_cursor = None;
            }
            if !typing && !navigable.is_empty() {
                let none = egui::Modifiers::NONE;
                let (down, up, space, right, left, find) = ui.input_mut(|inp| {
                    (
                        inp.consume_key(none, Key::ArrowDown) || inp.consume_key(none, Key::J),
                        inp.consume_key(none, Key::ArrowUp) || inp.consume_key(none, Key::K),
                        inp.consume_key(none, Key::Space),
                        inp.consume_key(none, Key::ArrowRight),
                        inp.consume_key(none, Key::ArrowLeft),
                        inp.consume_key(none, Key::F),
                    )
                });
                let pos = list_cursor.and_then(|(_, c)| navigable.iter().position(|&x| x == c));
                if down || up {
                    let at = match pos {
                        Some(p) if down => (p + 1).min(navigable.len() - 1),
                        Some(p) => p.saturating_sub(1),
                        None => next.and_then(|n| navigable.iter().position(|&x| x == n)).unwrap_or(0),
                    };
                    *list_cursor = Some((aid, navigable[at]));
                    cursor_moved = true;
                }
                if let Some((_, c)) = *list_cursor {
                    if space {
                        acts.push((c, RowAct::Toggle));
                    }
                    if right {
                        open_rows.insert((aid, c));
                    }
                    if left {
                        open_rows.remove(&(aid, c));
                    }
                    if find && web && !guide.steps[c].anchor.is_empty() {
                        acts.push((c, RowAct::Find));
                    }
                }
            }
            let cursor = list_cursor.map(|(_, c)| c);
            // a dashboard or search asked for an item
            let scroll_item = scroll_to_item.take_if(|(g, _)| *g == aid).map(|(_, i)| i);

            egui::ScrollArea::vertical().id_salt(("checklist", aid)).auto_shrink([false, false]).show(ui, |ui| {
                if shown == 0 {
                    ui.add_space(16.0);
                    let msg = if done == counted { "All done!" } else { "Nothing matches these filters" };
                    ui.vertical_centered(|ui| ui.label(RichText::new(msg).font(bold(18.0)).color(TEAL)));
                }
                let mut last_section: Option<String> = None;
                for (i, s) in guide.steps.iter().enumerate() {
                    if !visible(i, s) {
                        continue;
                    }
                    if last_section.as_deref() != Some(s.section.as_str()) {
                        last_section = Some(s.section.clone());
                        if !s.section.is_empty() {
                            let (d, n) = areas.iter().find(|a| a.0 == s.section).map_or((0, 0), |a| (a.1, a.2));
                            let shut = collapsed.contains(&(aid, s.section.clone()));
                            ui.add_space(4.0);
                            if section_header(ui, &s.section, d, n, shut).clicked() {
                                toggle_section = Some(s.section.clone());
                            }
                            ui.add_space(4.0);
                        }
                    }
                    if !s.section.is_empty() && collapsed.contains(&(aid, s.section.clone())) {
                        continue;
                    }
                    let is_next = next == Some(i);
                    let can_find = web && !s.anchor.is_empty();
                    // Off-screen rows take their remembered height instead of being laid out.
                    let w = ui.available_width();
                    let open = open_rows.contains(&(aid, i));
                    let thumb = if s.image.is_empty() || (compact && !open) || veiled(i) { None } else { thumb_of(&ctx, thumbs, &s.image) };
                    let look = RowLook { state: states[i], is_next, can_find, compact, open, cursor: cursor == Some(i), veiled: veiled(i), thumb };
                    let print = row_fingerprint(s, &look, w);
                    let wanted = scroll_item == Some(i);
                    if let Some(&(fp, h)) = row_heights.get(&(aid, i)) {
                        let r = Rect::from_min_size(ui.cursor().min, Vec2::new(w, h));
                        if fp == print && !(is_next && scroll_next) && !(cursor_moved && cursor == Some(i)) && !wanted && !ui.is_rect_visible(r) {
                            ui.allocate_space(Vec2::new(w, h));
                            ui.add_space(4.0);
                            continue;
                        }
                    }
                    let resp = step_row(ui, s, i, &look, &mut acts);
                    if cursor_moved && cursor == Some(i) {
                        resp.scroll_to_me(None);
                    }
                    if wanted {
                        resp.scroll_to_me(Some(Align::Center));
                    }
                    row_heights.insert((aid, i), (print, resp.rect.height()));
                    if is_next && scroll_next {
                        resp.scroll_to_me(Some(Align::Center));
                    }
                    ui.add_space(4.0);
                }
                ui.add_space(8.0);
                add_item_row(ui, guide, step_bufs.entry(aid).or_default());
            });
            if let Some(sec) = toggle_section {
                let key = (aid, sec);
                if !collapsed.remove(&key) {
                    collapsed.insert(key);
                }
            }
        }

        let mut find = None;
        let mut progressed = false;
        for (i, a) in acts {
            if i >= guide.steps.len() {
                continue;
            }
            match a {
                RowAct::Toggle => {
                    let s = &mut guide.steps[i];
                    s.done = !s.done;
                    if s.done {
                        s.missed = false;
                    }
                    progressed = true;
                }
                RowAct::Missed => {
                    let s = &mut guide.steps[i];
                    // an expired item can be un-missed only by ticking it; this toggles the manual flag
                    s.missed = !s.missed;
                    if s.missed {
                        s.done = false;
                    }
                    progressed = true;
                }
                RowAct::Optional => guide.steps[i].optional = !guide.steps[i].optional,
                RowAct::Details => {
                    if !open_rows.remove(&(aid, i)) {
                        open_rows.insert((aid, i));
                    }
                }
                RowAct::Find => {
                    let s = &guide.steps[i];
                    let page = if s.url.is_empty() { &guide.url } else { &s.url };
                    find = Some(links::with_text(page, &s.anchor));
                }
                RowAct::Delete => {
                    guide.steps.remove(i);
                }
                RowAct::TickBefore => {
                    guide.tick_before(i);
                    progressed = true;
                }
                RowAct::UntickFrom => {
                    guide.untick_from(i);
                    progressed = true;
                }
                RowAct::Up if i > 0 => guide.steps.swap(i, i - 1),
                RowAct::Down if i + 1 < guide.steps.len() => guide.steps.swap(i, i + 1),
                RowAct::Reveal => {
                    revealed.insert((aid, i));
                }
                RowAct::Note => *note_edit = Some((aid, i, guide.steps[i].note.clone())),
                RowAct::Image => {
                    let picked = rfd::FileDialog::new().set_title("Attach a screenshot or map").add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file();
                    if let Some(f) = picked {
                        guide.steps[i].image = f.display().to_string();
                    }
                }
                RowAct::NoImage => guide.steps[i].image.clear(),
                RowAct::OpenImage => {
                    let _ = open::that_detached(&guide.steps[i].image);
                }
                _ => {}
            }
        }
        if progressed {
            guide.follow_checklist(today);
        }
        if find.is_some() {
            guide.last_opened = Some(model::now());
        }
        if progressed || find.is_some() {
            game.last_guide = Some(aid);
            game.touch();
        }
        if paste {
            self.bulk = Some((aid, String::new()));
        }
        if let Some(url) = find {
            match open::that_detached(&url) {
                Ok(()) => self.set_status("Opened the guide at that item"),
                Err(e) => self.set_status_for(format!("Couldn't open the browser: {e}"), 6.0),
            }
        }
    }

    /// Picks up changes another copy of the app (or a text editor) saved to library.json,
    /// as long as there's nothing unsaved here.
    fn watch_file(&mut self, now: f64) {
        if now - self.last_file_check < 1.0 {
            return;
        }
        self.last_file_check = now;
        let m = mtime(&self.lib_path);
        if m.is_none() || m == self.file_mtime || self.last_hash != self.saved_hash {
            return;
        }
        self.file_mtime = m;
        if let Ok(lib) = model::read_file(&self.lib_path) {
            let h = hash_lib(&lib);
            if h != self.saved_hash {
                self.lib = lib;
                self.saved_hash = h;
                self.last_hash = h;
                self.set_status("Reloaded changes from library.json");
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Journal (right)

    // -----------------------------------------------------------------------------------------
    // Explorer (centre): the games as folders and tiles, like a file explorer window

    fn ex_go(&mut self, path: Vec<String>) {
        if path != self.ex_path {
            self.ex_back.push(std::mem::replace(&mut self.ex_path, path));
            self.ex_fwd.clear();
            self.ex_sel = None;
        }
    }

    fn ex_open_game(&mut self, id: u64) {
        self.selected = Some(id);
        self.explorer_open = false;
    }

    fn explorer_view(&mut self, ui: &mut Ui) {
        // ---- toolbar: back / forward / up, the path, search and view
        let mut go: Option<Vec<String>> = None;
        let (mut back, mut fwd) = (false, false);
        ui.horizontal(|ui| {
            if tool_arrow(ui, Vec2::new(-1.0, 0.0), !self.ex_back.is_empty(), "Back (Alt+Left)").clicked() {
                back = true;
            }
            if tool_arrow(ui, Vec2::new(1.0, 0.0), !self.ex_fwd.is_empty(), "Forward (Alt+Right)").clicked() {
                fwd = true;
            }
            if tool_arrow(ui, Vec2::new(0.0, -1.0), !self.ex_path.is_empty(), "Up one folder (Backspace)").clicked() {
                go = Some(self.ex_path[..self.ex_path.len() - 1].to_vec());
            }
            ui.add_space(4.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.selectable_label(self.ex_layout == ExplorerLayout::Details, "Details").clicked() {
                    self.ex_layout = ExplorerLayout::Details;
                }
                if ui.selectable_label(self.ex_layout == ExplorerLayout::Icons, "Large icons").clicked() {
                    self.ex_layout = ExplorerLayout::Icons;
                }
                ui.add(egui::TextEdit::singleline(&mut self.search).id_salt("explorer-search").hint_text("Search games…").desired_width(150.0));
                // the breadcrumb takes what's left
                let w = ui.available_width();
                ui.allocate_ui_with_layout(Vec2::new(w, ui.spacing().interact_size.y + 6.0), Layout::left_to_right(Align::Center), |ui| {
                    egui::Frame::new().fill(INK).stroke(Stroke::new(1.5f32, YELLOW)).corner_radius(2).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let (ir, _) = ui.allocate_exact_size(Vec2::new(18.0, 14.0), Sense::hover());
                        icons::folder_icon(ui.painter(), ir, true, true);
                        if crumb(ui, "Games", self.ex_path.is_empty()).clicked() {
                            go = Some(Vec::new());
                        }
                        for (i, part) in self.ex_path.iter().enumerate() {
                            ui.label(RichText::new(">").color(DIM));
                            if crumb(ui, part, i + 1 == self.ex_path.len()).clicked() {
                                go = Some(self.ex_path[..=i].to_vec());
                            }
                        }
                    });
                });
            });
        });
        if back {
            if let Some(p) = self.ex_back.pop() {
                self.ex_fwd.push(std::mem::replace(&mut self.ex_path, p));
                self.ex_sel = None;
            }
        }
        if fwd {
            if let Some(p) = self.ex_fwd.pop() {
                self.ex_back.push(std::mem::replace(&mut self.ex_path, p));
                self.ex_sel = None;
            }
        }
        if let Some(p) = go.take() {
            self.ex_go(p);
        }

        // ---- what's in this folder
        let ids = self.sorted_ids();
        let games: Vec<&Game> = ids.iter().filter_map(|id| self.lib.games.iter().find(|g| g.id == *id)).collect();
        let (folders, game_ids) = match explorer_children(&games, &self.ex_path) {
            Some(c) => c,
            None => {
                // the folder no longer exists (games moved or filtered away): back to the top
                self.ex_path.clear();
                explorer_children(&games, &[]).unwrap_or_default()
            }
        };
        let folder_tiles: Vec<(String, usize, Vec<Color32>)> = folders
            .iter()
            .map(|(name, ids)| {
                let peek = ids.iter().filter_map(|id| games.iter().find(|g| g.id == *id)).map(|g| platform_color(&g.platform)).collect();
                (name.clone(), ids.len(), peek)
            })
            .collect();
        let tiles: Vec<GameTile> = game_ids
            .iter()
            .filter_map(|id| games.iter().find(|g| g.id == *id))
            .map(|g| GameTile {
                id: g.id,
                title: g.title.clone(),
                platform: g.platform.clone(),
                status: g.status,
                folder: g.folder,
                progress: g.progress(),
                last_played: g.last_played,
                host: g.guides.first().map(|x| links::host(&x.url)).unwrap_or_default(),
            })
            .collect();
        drop(games);
        for t in &tiles {
            self.want_favicon(&t.host.clone());
            self.want_cover(t.id);
        }
        let boxart: Vec<Option<(egui::TextureId, Vec2)>> = tiles.iter().map(|t| self.cover_of(t.id)).collect();
        let covers: Vec<Option<egui::TextureId>> = tiles
            .iter()
            .map(|t| match self.favicons.get(&t.host) {
                Some(Fav::Ready(tex)) => Some(tex.id()),
                _ => None,
            })
            .collect();

        // ---- keyboard: Backspace up, Enter open, Alt+arrows back/forward
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        let mut open_item: Option<ExplorerItem> = None;
        if !typing {
            let (bksp, enter, alt_l, alt_r) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, Key::Backspace),
                    i.consume_key(egui::Modifiers::NONE, Key::Enter),
                    i.consume_key(egui::Modifiers::ALT, Key::ArrowLeft),
                    i.consume_key(egui::Modifiers::ALT, Key::ArrowRight),
                )
            });
            if bksp && !self.ex_path.is_empty() {
                go = Some(self.ex_path[..self.ex_path.len() - 1].to_vec());
            }
            if enter {
                open_item = self.ex_sel.clone();
            }
            if alt_l {
                if let Some(p) = self.ex_back.pop() {
                    self.ex_fwd.push(std::mem::replace(&mut self.ex_path, p));
                }
            }
            if alt_r {
                if let Some(p) = self.ex_fwd.pop() {
                    self.ex_back.push(std::mem::replace(&mut self.ex_path, p));
                }
            }
        }

        // ---- the items
        let mut acts: Vec<GameAct> = Vec::new();
        let mut select: Option<ExplorerItem> = None;
        let total = folder_tiles.len() + tiles.len();
        let sel = self.ex_sel.clone();
        let layout = self.ex_layout;
        let status_h = 22.0;
        let list_h = (ui.available_height() - status_h - 8.0).max(80.0);
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt(("explorer", self.ex_path.join("/"))).auto_shrink([false, false]).max_height(list_h).show(ui, |ui| {
            ui.set_min_height(list_h);
            if total == 0 {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| ui.label(RichText::new("This folder is empty.").color(DIM)));
                return;
            }
            match layout {
                ExplorerLayout::Icons => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                        for (name, count, peek) in &folder_tiles {
                            let item = ExplorerItem::Folder(name.clone());
                            let r = folder_tile(ui, name, *count, peek, sel.as_ref() == Some(&item));
                            if r.double_clicked() {
                                open_item = Some(item);
                            } else if r.clicked() {
                                select = Some(item);
                            }
                        }
                        for ((t, cover), art) in tiles.iter().zip(&covers).zip(&boxart) {
                            let item = ExplorerItem::Game(t.id);
                            let r = game_tile(ui, t, *cover, *art, sel.as_ref() == Some(&item));
                            if r.double_clicked() {
                                open_item = Some(item);
                            } else if r.clicked() {
                                select = Some(item);
                            }
                            r.context_menu(|ui| game_menu(ui, t.id, t.status, t.folder, &t.platform, &mut acts));
                        }
                    });
                }
                ExplorerLayout::Details => {
                    let w = ui.available_width();
                    details_header(ui, w);
                    for (n, (name, count, _)) in folder_tiles.iter().enumerate() {
                        let item = ExplorerItem::Folder(name.clone());
                        let r = details_row(ui, w, n, sel.as_ref() == Some(&item), None, name, &if *count == 1 { "Folder · 1 game".to_string() } else { format!("Folder · {count} games") }, "", None, "");
                        if r.double_clicked() {
                            open_item = Some(item);
                        } else if r.clicked() {
                            select = Some(item);
                        }
                    }
                    for (k, t) in tiles.iter().enumerate() {
                        let item = ExplorerItem::Game(t.id);
                        let played = t.last_played.map(model::ago).unwrap_or_else(|| "not yet".into());
                        let r = details_row(
                            ui,
                            w,
                            folder_tiles.len() + k,
                            sel.as_ref() == Some(&item),
                            Some(platform_color(&t.platform)),
                            &t.title,
                            &t.platform,
                            t.status.label(),
                            Some((t.progress, t.status.color())),
                            &played,
                        );
                        if r.double_clicked() {
                            open_item = Some(item);
                        } else if r.clicked() {
                            select = Some(item);
                        }
                        r.context_menu(|ui| game_menu(ui, t.id, t.status, t.folder, &t.platform, &mut acts));
                    }
                }
            }
        });

        // ---- status bar, like Explorer's
        ui.separator();
        ui.horizontal(|ui| {
            let items = if total == 1 { "1 item".to_string() } else { format!("{total} items") };
            ui.label(RichText::new(items).small().color(DIM));
            if let Some(ExplorerItem::Game(id)) = &self.ex_sel {
                if let Some(t) = tiles.iter().find(|t| t.id == *id) {
                    ui.label(RichText::new(format!("·  {} selected: {} · {:.0}%  (double-click or Enter to open)", t.title, t.status.label(), t.progress * 100.0)).small().color(TEAL));
                }
            } else if let Some(ExplorerItem::Folder(f)) = &self.ex_sel {
                ui.label(RichText::new(format!("·  {f} selected  (double-click or Enter to open)")).small().color(TEAL));
            }
        });

        if let Some(s) = select {
            self.ex_sel = Some(s);
        }
        match open_item {
            Some(ExplorerItem::Folder(name)) => {
                let mut p = self.ex_path.clone();
                p.push(name);
                self.ex_go(p);
            }
            Some(ExplorerItem::Game(id)) => self.ex_open_game(id),
            None => {}
        }
        if let Some(p) = go {
            self.ex_go(p);
        }
        self.apply_game_acts(acts);
    }

    fn apply_game_acts(&mut self, acts: Vec<GameAct>) {
        for a in acts {
            match a {
                GameAct::Select(id) => {
                    self.selected = Some(id);
                    self.explorer_open = false;
                }
                GameAct::Status(id, s) => {
                    if let Some(g) = self.lib.game_mut(id) {
                        g.status = s;
                    }
                }
                GameAct::Folder(id, f) => {
                    if let Some(g) = self.lib.game_mut(id) {
                        g.folder = f;
                    }
                }
            }
        }
    }

    fn resume_view(&mut self, ui: &mut Ui, gid: u64) {
        let Some(g) = self.lib.game_mut(gid) else { return };
        ui.add(
            egui::TextEdit::multiline(&mut g.resume_note)
                .id_salt(("note", gid))
                .hint_text("Where are you, and what were you about to do?\ne.g. Just beat the 2nd boss. Heading to the swamp for the last 3 feathers.")
                .desired_rows(6)
                .desired_width(f32::INFINITY),
        );
        let mut logged = false;
        // wraps instead of overflowing when the panel is narrow
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!g.resume_note.trim().is_empty(), egui::Button::new("Save to log"))
                .on_hover_text("Copy this note into the session log with today's date")
                .clicked()
            {
                g.log.push(LogEntry { at: model::now(), text: g.resume_note.trim().to_string() });
                g.touch();
                logged = true;
            }
            if ui.button("Played today").on_hover_text("Mark this game as played just now").clicked() {
                g.touch();
            }
        });
        if logged {
            self.set_status("Saved to the session log");
        }
    }

    fn log_view(&mut self, ui: &mut Ui, gid: u64) {
        let Some(g) = self.lib.game_mut(gid) else { return };
        let (_, add, enter) = input_row(ui, &mut self.new_log, "What happened this session?", ("new-log", gid), "Add");
        if (add || enter) && !self.new_log.trim().is_empty() {
            g.log.push(LogEntry { at: model::now(), text: std::mem::take(&mut self.new_log).trim().to_string() });
            g.touch();
        }
        ui.separator();
        if g.log.is_empty() {
            ui.label(RichText::new("Nothing logged yet. Each entry is dated, so you can see what you did last time.").small().color(DIM));
            ui.allocate_space(ui.available_size());
            return;
        }
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt(("log", gid)).auto_shrink([false, false]).show(ui, |ui| {
            for (i, e) in g.log.iter().enumerate().rev() {
                let r = ui
                    .vertical(|ui| {
                        ui.label(RichText::new(model::date_time(e.at)).small().color(TEAL));
                        ui.add(egui::Label::new(RichText::new(&e.text).color(YELLOW)).wrap());
                    })
                    .response;
                r.context_menu(|ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(e.text.clone());
                        ui.close_menu();
                    }
                    if ui.button("Delete entry").clicked() {
                        remove = Some(i);
                        ui.close_menu();
                    }
                });
                ui.add_space(6.0);
            }
        });
        if let Some(i) = remove {
            g.log.remove(i);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Taskbar (bottom)

    fn taskbar(&mut self, ui: &mut Ui) {
        let mut go = false;
        ui.horizontal_centered(|ui| {
            let cont = self.game().and_then(|g| g.continue_guide().map(|x| (g.title.clone(), x.title.clone(), x.spot())));
            let (r, resp) = ui.allocate_exact_size(Vec2::new(118.0, 28.0), Sense::click());
            let enabled = cont.is_some();
            let (bg, fg) = if !enabled {
                (INK, DIM)
            } else if resp.is_pointer_button_down_on() {
                (TEAL, INK)
            } else if resp.hovered() {
                (INDIGO, TEAL)
            } else {
                (INK, YELLOW)
            };
            let p = ui.painter();
            p.rect_filled(r, 2.0, bg);
            p.rect_stroke(r, 2.0, Stroke::new(1.5f32, if enabled { YELLOW } else { DIM }), StrokeKind::Inside);
            let t = Pos2::new(r.left() + 14.0, r.center().y);
            p.add(egui::Shape::convex_polygon(vec![t + Vec2::new(-4.0, -6.0), t + Vec2::new(6.0, 0.0), t + Vec2::new(-4.0, 6.0)], fg, Stroke::NONE));
            p.text(Pos2::new(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, "Continue", bold(16.0), fg);
            if enabled {
                glow_rect(p, r, 2.0, YELLOW, 4.0);
            }
            if resp.on_hover_text("Reopen the last guide at your saved spot and start the play timer (Enter)").clicked() && enabled {
                go = true;
            }
            let mini = egui::Button::new(RichText::new("Mini").color(if self.mini_open { INK } else { YELLOW })).fill(if self.mini_open { YELLOW } else { INK }).stroke(Stroke::new(1.5f32, YELLOW));
            if ui.add(mini).on_hover_text("A small always-on-top window with your next item, to tick while you play (Ctrl+Alt+M)").clicked() {
                self.mini_open = !self.mini_open;
            }
            if let Some(p) = self.playing() {
                let secs = p.session_start.map_or(0, |s| (model::now() - s).max(0) as u64);
                let label = format!("\u{25a0} {}", model::fmt_clock(secs));
                let tip = format!("Playing {}: click to stop the timer", p.title);
                let id = p.id;
                let b = egui::Button::new(RichText::new(label).color(YELLOW)).fill(INK).stroke(Stroke::new(1.5f32, YELLOW));
                if ui.add(b).on_hover_text(tip).clicked() {
                    self.stop_playing(id);
                }
            }
            vline(ui);

            let total_guides: usize = self.lib.games.iter().map(|g| g.guides.len()).sum();
            let plural = |n: usize, word: &str| if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") };
            let summary = format!("{} · {}", plural(self.lib.games.len(), "game"), plural(total_guides, "guide"));
            let sw = ui.painter().layout_no_wrap(summary.clone(), pixel(16.0), YELLOW).size().x + 30.0;
            let (tr, _) = ui.allocate_exact_size(Vec2::new((ui.available_width() - sw).max(50.0), 28.0), Sense::hover());
            let text = match &cont {
                Some((game, guide, spot)) if spot.is_empty() => format!("{game}  ▸  {guide}"),
                Some((game, guide, spot)) => format!("{game}  ▸  {guide}  ▸  {spot}"),
                None => match self.game() {
                    Some(g) => format!("{}: add a guide to pick up where you left off", g.title),
                    None => "Add a game to get started".into(),
                },
            };
            ui.painter().with_clip_rect(tr).text(tr.left_center() + Vec2::new(4.0, 0.0), Align2::LEFT_CENTER, text, pixel(16.0), INK);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                egui::Frame::new().fill(INK).stroke(Stroke::new(2.0f32, YELLOW)).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                    ui.label(RichText::new(summary).color(YELLOW));
                });
            });
        });
        if go {
            self.continue_game();
        }
    }

    // -----------------------------------------------------------------------------------------
    // Windows

    fn add_game_window(&mut self, ctx: &egui::Context) {
        let Some(ng) = &mut self.add_game else { return };
        let mut open = true;
        let mut create = false;
        let mut cancelled = false;
        egui::Window::new(RichText::new("Add game").font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                egui::Grid::new("add-game").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(RichText::new("Title").color(TEAL));
                    let r = ui.add(egui::TextEdit::singleline(&mut ng.title).hint_text("e.g. Elden Ring").desired_width(240.0));
                    if ng.focus {
                        r.request_focus();
                        ng.focus = false;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        create = true;
                    }
                    ui.end_row();
                    ui.label(RichText::new("Platform").color(TEAL));
                    let r = ui.add(egui::TextEdit::singleline(&mut ng.platform).hint_text("PS5, Switch, PC…").desired_width(240.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        create = true;
                    }
                    ui.end_row();
                    ui.label(RichText::new("Status").color(TEAL));
                    ui.horizontal_wrapped(|ui| {
                        for s in Status::ALL {
                            ui.selectable_value(&mut ng.status, s, RichText::new(s.label()));
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Add game").clicked() {
                        create = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });
        if create {
            if let Some(ng) = self.add_game.take() {
                self.create_game(ng.title, ng.platform, ng.status);
            }
        } else if !open || cancelled || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.add_game = None;
        }
    }

    fn bulk_window(&mut self, ctx: &egui::Context) {
        let Some((guide_id, _)) = &self.bulk else { return };
        let guide_id = *guide_id;
        let (url, existing, web) = self
            .lib
            .games
            .iter()
            .flat_map(|g| &g.guides)
            .find(|g| g.id == guide_id)
            .map_or((String::new(), 0, false), |g| (g.url.clone(), g.steps.len(), g.kind != Kind::Video && g.kind != Kind::Sheet && !links::is_local(&g.url)));

        // parse once per change of text/mode, not every frame (fetched pages can be big)
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.bulk.as_ref().map(|b| &b.1).hash(&mut h);
            self.bulk_headings.hash(&mut h);
            h.finish()
        };
        if self.bulk_cache.as_ref().is_none_or(|c| c.0 != key) {
            let text = &self.bulk.as_ref().unwrap().1;
            let parsed = if self.bulk_headings { checklist::parse_headings(text) } else { checklist::parse(text) };
            self.bulk_cache = Some((key, parsed));
        }
        let has_headings = self.bulk.as_ref().is_some_and(|b| b.1.lines().any(|l| l.starts_with("## ")));

        let mut open = true;
        let mut apply = false;
        let mut fetch = false;
        let mut open_file = false;
        let parsed = &self.bulk_cache.as_ref().unwrap().1;
        let n = parsed.len();
        egui::Window::new(RichText::new("Paste or fetch checklist").font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .default_width(600.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open file…").on_hover_text("Read the checklist from a saved page or text file").clicked() {
                        open_file = true;
                    }
                    let label = if self.bulk_fetching { "Fetching…" } else { "Fetch from guide page" };
                    if ui
                        .add_enabled(web && !self.bulk_fetching && !url.is_empty(), egui::Button::new(RichText::new(label).font(bold(16.0))))
                        .on_hover_text(format!("Download the guide's text from\n{url}"))
                        .clicked()
                    {
                        fetch = true;
                    }
                    ui.label(RichText::new("or paste the guide's text below").color(DIM));
                });
                if let Some(err) = &self.bulk_error {
                    ui.label(RichText::new(err).color(MAGENTA));
                }
                ui.label(
                    RichText::new("Area headings, chests, objectives, achievements and quests (with their How:, Reward: and End: lines) are picked up automatically. A plain list works too: one item per line.")
                        .small()
                        .color(DIM),
                );
                ui.add_space(4.0);
                let text = &mut self.bulk.as_mut().unwrap().1;
                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_rows(15)
                            .desired_width(f32::INFINITY)
                            .hint_text("Castle Town (1st Playthrough)\nChest 1 - Potion; on the road to ...\nObjective 14: Frozen Rail Ride\n\"Use ice-based magic to ...\"\nHow: ...\nReward: ..."),
                    );
                });
                ui.add_space(4.0);
                if has_headings || self.bulk_headings {
                    ui.checkbox(&mut self.bulk_headings, "Use only the page's headings as items")
                        .on_hover_text("For long walkthroughs written as prose: one item per chapter / section heading.");
                }
                // preview of what was understood
                let mut counts: Vec<String> = StepKind::ALL
                    .iter()
                    .filter_map(|k| {
                        let c = parsed.iter().filter(|s| s.kind == *k).count();
                        (c > 0).then(|| format!("{c} {}", k.plural().to_lowercase()))
                    })
                    .collect();
                let optional = parsed.iter().filter(|s| s.optional).count();
                if optional > 0 {
                    counts.push(format!("{optional} optional with deadlines"));
                }
                let mut sections: Vec<&str> = parsed.iter().map(|s| s.section.as_str()).filter(|s| !s.is_empty()).collect();
                sections.dedup();
                if !sections.is_empty() {
                    counts.push(format!("in {} areas", sections.len()));
                }
                let summary = if parsed.is_empty() { "Nothing found yet".to_string() } else { format!("Found {}", counts.join(", ")) };
                ui.label(RichText::new(summary).color(TEAL));
                if n > 300 || (n > 150 && has_headings && !self.bulk_headings) {
                    ui.label(RichText::new("That's a lot of items. If it's picking up every paragraph, try \"Use only the page's headings\", or trim the text first.").small().color(MAGENTA));
                }
                if existing > 0 {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.bulk_replace, false, "Add to the end");
                        ui.radio_value(&mut self.bulk_replace, true, format!("Replace the current checklist ({existing} items)"));
                    });
                }
                if ui.add_enabled(n > 0, egui::Button::new(RichText::new(format!("Add {n} items")).font(bold(16.0)))).clicked() {
                    apply = true;
                }
            });
        if open_file {
            let picked = rfd::FileDialog::new()
                .set_title("Open a guide's text")
                .add_filter("Saved pages and text", &["html", "htm", "txt", "md", "csv"])
                .add_filter("All files", &["*"])
                .pick_file();
            if let Some(path) = picked {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let raw = String::from_utf8_lossy(&bytes).into_owned();
                        let html = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"));
                        if let Some((_, buf)) = &mut self.bulk {
                            *buf = if html { online::html_to_text(&raw) } else { raw };
                        }
                        self.bulk_error = None;
                    }
                    Err(e) => self.bulk_error = Some(format!("Couldn't read {}: {e}", path.display())),
                }
            }
        }
        if fetch {
            self.bulk_fetching = true;
            self.bulk_error = None;
            self.online.send(Job::PageText { guide: guide_id, url });
        }
        if apply {
            // keep the text for reading beside the checklist
            if let Some((_, text)) = &self.bulk {
                if text.trim().len() > 40 {
                    if let Err(e) = model::save_page(&self.pages_dir, guide_id, text) {
                        self.set_status_for(format!("Couldn't keep the guide's text: {e}"), 6.0);
                    }
                    if self.reader.as_ref().is_some_and(|r| r.guide == guide_id) {
                        self.reader = None;
                    }
                }
            }
            let parsed = self.bulk_cache.take().map(|c| c.1).unwrap_or_default();
            let replace = self.bulk_replace;
            for game in &mut self.lib.games {
                let today = game.today();
                if let Some(g) = game.guides.iter_mut().find(|g| g.id == guide_id) {
                    if replace {
                        g.steps.clear();
                    }
                    g.steps.extend(parsed);
                    g.follow_checklist(today);
                    break;
                }
            }
            self.active_list = Some(guide_id);
            self.close_bulk();
            self.set_status(format!("Added {n} items"));
        } else if !open {
            self.close_bulk();
        }
    }

    fn search_all_window(&mut self, ctx: &egui::Context) {
        let Some(query) = &mut self.search_all else { return };
        let mut open = true;
        let mut chosen: Option<SearchHit> = None;
        let q = query.trim().to_lowercase();
        let hits = if q.chars().count() >= 2 { search_library(&self.lib, &q, 200) } else { Vec::new() };
        egui::Window::new(RichText::new("Search everything").font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .default_width(640.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(query)
                        .id_salt("search-all")
                        .hint_text("Search every game, guide and checklist item: \"Martira\", \"Gold Beetle\", \"Paralogue\"...")
                        .desired_width(f32::INFINITY),
                );
                if !r.has_focus() && q.is_empty() {
                    r.request_focus();
                }
                if q.chars().count() < 2 {
                    ui.label(RichText::new("Type at least 2 letters.").small().color(DIM));
                    return;
                }
                let more = if hits.len() >= 200 { " (showing the first 200)" } else { "" };
                ui.label(RichText::new(format!("{} results{more}", hits.len())).color(TEAL));
                egui::ScrollArea::vertical().id_salt("search-all-results").max_height(420.0).show(ui, |ui| {
                    for h in &hits {
                        let resp = egui::Frame::new().fill(CARD_BG).corner_radius(2).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(RichText::new(&h.title).font(bold(15.0)).color(if h.done { DIM } else { WHITE }));
                            ui.label(RichText::new(&h.place).small().color(TEAL));
                            if !h.snippet.is_empty() {
                                ui.label(RichText::new(&h.snippet).small().color(DIM));
                            }
                        });
                        let resp = resp.response.interact(Sense::click()).on_hover_text("Open it");
                        if resp.hovered() {
                            ui.painter().rect_stroke(resp.rect, 2.0, Stroke::new(1.0f32, TEAL), StrokeKind::Inside);
                        }
                        if resp.clicked() {
                            chosen = Some(h.clone());
                        }
                        ui.add_space(3.0);
                    }
                });
            });
        let term = q.clone();
        if let Some(h) = chosen {
            // filter the checklist to what was searched for
            let search = if h.item.is_some() { term } else { String::new() };
            self.go_to(h.game, h.guide, h.item, search);
            self.search_all = None;
        } else if !open {
            self.search_all = None;
        }
    }

    /// Ticks the achievements a platform says you've earned. A game with no achievement checklist
    /// gets one made from the platform's list. Returns a summary for the user.
    fn apply_achievements(&mut self, game: u64, source: Source, list: &[accounts::Achievement]) -> String {
        let new_id = self.lib.new_id();
        let Some(g) = self.lib.game_mut(game) else { return "That game is gone.".into() };
        let earned = list.iter().filter(|a| a.achieved).count();
        if let Some(gi) = g.trophy_guide() {
            let guide = &mut g.guides[gi];
            let mut ticked = 0;
            let mut unmatched = 0;
            for a in list {
                let k = accounts::key(&a.name);
                let hit = guide.steps.iter_mut().find(|s| {
                    s.kind == StepKind::Achievement && (accounts::key(&s.text) == k || s.text.split_once(": ").is_some_and(|(_, r)| accounts::key(r) == k))
                });
                match hit {
                    Some(s) => {
                        if a.rarity.is_some() {
                            s.rarity = a.rarity;
                        }
                        if a.achieved {
                            if !s.done {
                                ticked += 1;
                            }
                            s.done = true;
                        }
                    }
                    None => unmatched += 1,
                }
            }
            let title = guide.title.clone();
            let mut msg = format!("{}: {earned} of {} earned; ticked {ticked} new in \"{title}\".", source.label(), list.len());
            if unmatched > 0 {
                msg.push_str(&format!(" {unmatched} {} names didn't match an item.", source.label()));
            }
            msg
        } else {
            let url = match source {
                Source::Steam => format!("https://steamcommunity.com/stats/{}/achievements", g.steam_app.trim()),
                Source::Xbox => "https://www.xbox.com/en-US/play/achievements".to_string(),
            };
            let steps: Vec<Step> = list
                .iter()
                .map(|a| Step { kind: StepKind::Achievement, detail: a.description.clone(), done: a.achieved, section: "Achievements".into(), anchor: a.name.clone(), rarity: a.rarity, ..Step::task(a.name.clone()) })
                .collect();
            let mut guide = Guide::new(new_id, url);
            guide.title = format!("{} achievements ({})", source.label(), g.title);
            guide.auto_title = false;
            guide.kind = Kind::Trophy;
            guide.steps = steps;
            g.guides.push(guide);
            format!("{}: made an achievement checklist with {} items, {earned} already earned.", source.label(), list.len())
        }
    }

    fn sync_window(&mut self, ctx: &egui::Context) {
        let Some(gid) = self.sync_window else { return };
        let Some(game) = self.lib.games.iter().find(|g| g.id == gid) else {
            self.sync_window = None;
            return;
        };
        let title = game.title.clone();
        let (mut steam_app, mut xbox_title) = (game.steam_app.clone(), game.xbox_title.clone());
        let steam_ready = !self.steam_key.trim().is_empty() && !self.steam_id.is_empty();
        let xbox_ready = !self.xbox_key.trim().is_empty();
        let mut open = true;
        let mut jobs: Vec<Job> = Vec::new();
        let mut to_settings = false;
        egui::Window::new(RichText::new(format!("Sync achievements: {title}")).font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .default_width(520.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Earned achievements get ticked in this game's trophy checklist. If it has none, one is made from the platform's list. Nothing gets unticked.")
                        .small()
                        .color(DIM),
                );
                let section = |ui: &mut Ui, label: &str| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(label).font(bold(16.0)).color(TEAL));
                };
                section(ui, "Steam");
                if steam_ready {
                    ui.horizontal(|ui| {
                        ui.label("App ID");
                        ui.add(egui::TextEdit::singleline(&mut steam_app).id_salt("steam-app").desired_width(90.0).hint_text("e.g. 2679460"));
                        if ui.button("Find on Steam").clicked() {
                            jobs.push(Job::FindGame { game: gid, source: Source::Steam, key: String::new(), title: title.clone() });
                        }
                        if ui.add_enabled(!steam_app.trim().is_empty(), egui::Button::new(RichText::new("Sync from Steam").font(bold(15.0)))).clicked() {
                            jobs.push(Job::Sync { game: gid, source: Source::Steam, key: self.steam_key.clone(), user: self.steam_id.clone(), id: steam_app.clone() });
                        }
                    });
                    ui.label(RichText::new(format!("As {}", self.steam_name)).small().color(DIM));
                } else if ui.link("Add your Steam key and profile in Settings \u{25b8} Accounts").clicked() {
                    to_settings = true;
                }
                section(ui, "Xbox (OpenXBL)");
                if xbox_ready {
                    ui.horizontal(|ui| {
                        ui.label("Title ID");
                        ui.add(egui::TextEdit::singleline(&mut xbox_title).id_salt("xbox-title").desired_width(110.0).hint_text("found for you"));
                        if ui.button("Find on Xbox").clicked() {
                            jobs.push(Job::FindGame { game: gid, source: Source::Xbox, key: self.xbox_key.clone(), title: title.clone() });
                        }
                        if ui.add_enabled(!xbox_title.trim().is_empty(), egui::Button::new(RichText::new("Sync from Xbox").font(bold(15.0)))).clicked() {
                            jobs.push(Job::Sync { game: gid, source: Source::Xbox, key: self.xbox_key.clone(), user: String::new(), id: xbox_title.clone() });
                        }
                    });
                } else if ui.link("Add your OpenXBL key in Settings \u{25b8} Accounts").clicked() {
                    to_settings = true;
                }
                ui.add_space(6.0);
                ui.label(RichText::new("PlayStation has no public trophy API: copy your trophy list (PSNProfiles or the PS5) and use Checklist \u{25b8} \u{2026} \u{25b8} Tick from a list.").small().color(DIM));
                if !self.sync_note.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(&self.sync_note).color(YELLOW));
                }
            });
        if let Some(g) = self.lib.game_mut(gid) {
            g.steam_app = steam_app.trim().to_string();
            g.xbox_title = xbox_title.trim().to_string();
        }
        if !jobs.is_empty() && !self.online_on {
            self.sync_note = "Turn Online on to sync achievements.".into();
            jobs.clear();
        }
        for j in jobs {
            self.sync_note = "Working\u{2026}".into();
            self.online.send(j);
        }
        if to_settings {
            self.show_settings = true;
        }
        if !open {
            self.sync_window = None;
            self.sync_note.clear();
        }
    }

    /// Every game's trophy progress in one place, closest to complete first.
    fn trophies_view(&mut self, ui: &mut Ui) {
        struct Row {
            game: u64,
            guide: u64,
            title: String,
            platform: String,
            done: usize,
            total: usize,
            /// (name, missable, rarity)
            left: Vec<(String, bool, Option<f32>)>,
            played: u64,
        }
        let at = model::now();
        let mut rows: Vec<Row> = self
            .lib
            .games
            .iter()
            .filter_map(|g| {
                let gd = &g.guides[g.trophy_guide()?];
                let ach: Vec<&Step> = gd.steps.iter().filter(|s| s.kind == StepKind::Achievement && !s.optional).collect();
                Some(Row {
                    game: g.id,
                    guide: gd.id,
                    title: g.title.clone(),
                    platform: g.platform.clone(),
                    done: ach.iter().filter(|s| s.done).count(),
                    total: ach.len(),
                    left: {
                        let mut v: Vec<(String, bool, Option<f32>)> = ach.iter().filter(|s| !s.done).map(|s| (s.text.clone(), s.is_missable(), s.rarity)).collect();
                        // easiest (most players have it) first, when the platform says
                        v.sort_by(|a, b| b.2.unwrap_or(-1.0).total_cmp(&a.2.unwrap_or(-1.0)));
                        v
                    },
                    played: g.played(at),
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            let fa = a.done as f32 / a.total.max(1) as f32;
            let fb = b.done as f32 / b.total.max(1) as f32;
            // unfinished first, closest to done at the top; finished at the bottom
            (a.done == a.total).cmp(&(b.done == b.total)).then(fb.total_cmp(&fa))
        });
        let mut open: Option<(u64, u64)> = None;
        let mut sync: Option<u64> = None;
        let earned: usize = rows.iter().map(|r| r.done).sum();
        let total: usize = rows.iter().map(|r| r.total).sum();
        ui.label(RichText::new(format!("{earned} of {total} trophies & achievements across {} games", rows.len())).font(bold(18.0)).color(YELLOW));
        if rows.is_empty() {
            ui.label(RichText::new("No game has a trophy checklist yet. Add a trophy guide, or use Sync achievements on a game.").color(DIM));
        }
        ui.add_space(6.0);
        egui::ScrollArea::vertical().id_salt("trophies").auto_shrink([false, false]).show(ui, |ui| {
            for r in &rows {
                egui::Frame::new().fill(CARD_BG).stroke(Stroke::new(1.0f32, if r.done == r.total { GREEN } else { INDIGO })).corner_radius(3).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                        icons::trophy(ui.painter(), ir, if r.done == r.total { GREEN } else { YELLOW });
                        ui.label(RichText::new(&r.title).font(bold(17.0)).color(WHITE));
                        ui.label(RichText::new(&r.platform).small().color(DIM));
                        if r.played >= 60 {
                            ui.label(RichText::new(format!("\u{b7} played {}", model::fmt_duration(r.played))).small().color(DIM));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Sync\u{2026}").on_hover_text("Pull earned achievements from Steam or Xbox").clicked() {
                                sync = Some(r.game);
                            }
                            if ui.button("Open checklist").clicked() {
                                open = Some((r.game, r.guide));
                            }
                            ui.label(RichText::new(format!("{} / {}", r.done, r.total)).font(bold(16.0)).color(if r.done == r.total { GREEN } else { YELLOW }));
                        });
                    });
                    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
                    rail(ui, bar.shrink2(Vec2::new(0.0, 1.0)), r.done as f32 / r.total.max(1) as f32, false, ("trophy-bar", r.game));
                    if !r.left.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let sorted = r.left.iter().any(|l| l.2.is_some());
                            ui.label(RichText::new(format!("{} left{}:", r.left.len(), if sorted { ", easiest first" } else { "" })).small().color(TEAL));
                            for (name, missable, rarity) in r.left.iter().take(12) {
                                let text = match rarity {
                                    Some(p) => format!("{name} ({p:.0}%)"),
                                    None => name.clone(),
                                };
                                tag(ui, &text, if *missable { MAGENTA } else { DIM });
                            }
                            if r.left.len() > 12 {
                                ui.label(RichText::new(format!("+{} more", r.left.len() - 12)).small().color(DIM));
                            }
                        });
                    }
                });
                ui.add_space(6.0);
            }
        });
        if let Some((game, guide)) = open {
            self.selected = Some(game);
            self.active_list = Some(guide);
            self.trophies_open = false;
            self.left_view = false;
            self.list_section = None;
            self.list_search.clear();
        }
        if let Some(game) = sync {
            self.sync_window = Some(game);
            self.sync_note.clear();
        }
    }

    /// Everything that can still be missed, across games: due soon first, then by deadline, then
    /// items the guides call missable.
    fn missables_view(&mut self, ui: &mut Ui) {
        struct Item {
            guide: u64,
            idx: usize,
            text: String,
            place: String,
            state: StepState,
            deadline: String,
            /// Days until the deadline (in-game), for sorting.
            days: i64,
        }
        let include_all = self.missables_all;
        let mut games: Vec<&Game> = self.lib.games.iter().filter(|g| include_all || !matches!(g.status, Status::Completed | Status::Dropped)).collect();
        games.sort_by(|a, b| a.status.rank().cmp(&b.status.rank()).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())));
        let mut groups: Vec<(u64, String, String, String, Vec<Item>)> = Vec::new();
        for g in games {
            let today = g.today();
            let mut items = Vec::new();
            for gd in &g.guides {
                let next = gd.next_index(today);
                for (i, s) in gd.steps.iter().enumerate() {
                    if !s.is_open(today) || !s.is_missable() {
                        continue;
                    }
                    let hidden = g.spoiler_guard && next.is_some_and(|n| i > n) && !self.revealed.contains(&(gd.id, i));
                    let days = match (today, model::parse_md(&s.deadline)) {
                        (Some(t), Some(d)) => (d.0 as i64 * 31 + d.1 as i64) - (t.0 as i64 * 31 + t.1 as i64),
                        (None, Some(d)) => d.0 as i64 * 31 + d.1 as i64,
                        _ => i64::MAX,
                    };
                    let place = if s.section.is_empty() { gd.title.clone() } else { format!("{} \u{203a} {}", gd.title, s.section) };
                    items.push(Item {
                        guide: gd.id,
                        idx: i,
                        text: if hidden { "\u{2592}\u{2592}\u{2592}\u{2592}  (spoiler guard)".into() } else { s.text.clone() },
                        place,
                        state: s.state(today),
                        deadline: s.deadline.clone(),
                        days,
                    });
                }
            }
            if items.is_empty() {
                continue;
            }
            items.sort_by(|a, b| (b.state == StepState::DueSoon).cmp(&(a.state == StepState::DueSoon)).then(a.days.cmp(&b.days)));
            groups.push((g.id, g.title.clone(), g.platform.clone(), g.game_date.clone(), items));
        }
        let total: usize = groups.iter().map(|g| g.4.len()).sum();
        let due: usize = groups.iter().flat_map(|g| &g.4).filter(|i| i.state == StepState::DueSoon).count();
        ui.horizontal(|ui| {
            let mut head = format!("{total} things you could still miss, in {} games", groups.len());
            if due > 0 {
                head.push_str(&format!(" \u{b7} {due} due soon"));
            }
            ui.label(RichText::new(head).font(bold(18.0)).color(YELLOW));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.checkbox(&mut self.missables_all, "Include finished and dropped games");
            });
        });
        ui.label(RichText::new("Items with a deadline, or that a guide calls missable. Set a game's In-game date to see what's due soon.").small().color(DIM));
        ui.add_space(6.0);
        let mut tick: Option<(u64, u64, usize)> = None;
        let mut show: Option<(u64, u64, usize)> = None;
        egui::ScrollArea::vertical().id_salt("missables").auto_shrink([false, false]).show(ui, |ui| {
            if groups.is_empty() {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| ui.label(RichText::new("Nothing missable left. Nice.").font(bold(18.0)).color(GREEN)));
            }
            for (gid, title, platform, date, items) in &groups {
                egui::Frame::new().fill(CARD_BG).stroke(Stroke::new(1.0f32, INDIGO)).corner_radius(3).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).font(bold(17.0)).color(WHITE));
                        ui.label(RichText::new(platform).small().color(DIM));
                        if !date.is_empty() {
                            ui.label(RichText::new(format!("\u{b7} in-game date {date}")).small().color(TEAL));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{}", items.len())).font(bold(16.0)).color(YELLOW));
                        });
                    });
                    for it in items.iter().take(40) {
                        ui.horizontal(|ui| {
                            if big_checkbox(ui, false).on_hover_text("Tick off").clicked() {
                                tick = Some((*gid, it.guide, it.idx));
                            }
                            let col = if it.state == StepState::DueSoon { MAGENTA } else { WHITE };
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Show").on_hover_text("Open it in the checklist").clicked() {
                                    show = Some((*gid, it.guide, it.idx));
                                }
                                match it.state {
                                    StepState::DueSoon => tag(ui, &format!("Ends {}", it.deadline), MAGENTA),
                                    _ if !it.deadline.is_empty() => tag(ui, &format!("Ends {}", it.deadline), DIM),
                                    _ => tag(ui, "missable", DIM),
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add(egui::Label::new(RichText::new(&it.text).color(col)).truncate());
                                    ui.add(egui::Label::new(RichText::new(&it.place).small().color(DIM)).truncate());
                                });
                            });
                        });
                    }
                    if items.len() > 40 {
                        ui.label(RichText::new(format!("+{} more", items.len() - 40)).small().color(DIM));
                    }
                });
                ui.add_space(6.0);
            }
        });
        if let Some((gid, lid, i)) = tick {
            if let Some(g) = self.lib.game_mut(gid) {
                let today = g.today();
                if let Some(gd) = g.guides.iter_mut().find(|x| x.id == lid) {
                    if let Some(s) = gd.steps.get_mut(i) {
                        s.done = true;
                        s.missed = false;
                        let t = s.text.clone();
                        gd.follow_checklist(today);
                        g.touch();
                        self.set_status(format!("Ticked {t}"));
                    }
                }
            }
        }
        if let Some((gid, lid, i)) = show {
            self.go_to(gid, Some(lid), Some(i), String::new());
        }
    }

    /// The mini window's contents: the next item of the current checklist, with Tick.
    fn mini_contents(&mut self, ui: &mut Ui) {
        enum MiniAct {
            Tick,
            Untick,
            Find(String),
            Focus,
        }
        let mut act = None;
        let Some((gid, lid)) = self.current_list() else {
            ui.label(RichText::new("Pick a game with a checklist in JAGT.").color(TEAL));
            return;
        };
        let Some(game) = self.lib.games.iter().find(|g| g.id == gid) else { return };
        let Some(guide) = game.guides.iter().find(|g| g.id == lid) else { return };
        let today = game.today();
        let done = guide.steps.iter().filter(|s| s.done).count();
        let counted = guide.steps.iter().filter(|s| !s.missed).count().max(1);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(&game.title).font(bold(16.0)).color(YELLOW)).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{done}/{counted}")).color(TEAL));
                if let Some(s) = game.session_start {
                    ui.label(RichText::new(model::fmt_clock((model::now() - s).max(0) as u64)).color(MAGENTA));
                }
            });
        });
        match guide.next_index(today).map(|i| &guide.steps[i]) {
            Some(s) => {
                if !s.section.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&s.section).small().color(DIM)).truncate());
                }
                ui.add(egui::Label::new(RichText::new(&s.text).font(bold(18.0)).color(WHITE)).wrap());
                for t in [&s.detail, &s.how, &s.note] {
                    if !t.is_empty() {
                        ui.add(egui::Label::new(RichText::new(t).small().color(DIM)).truncate()).on_hover_text(t.as_str());
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button(RichText::new("\u{2714} Tick").font(bold(16.0)).color(GREEN)).on_hover_text("Ctrl+Alt+Space, even while the game has the focus").clicked() {
                        act = Some(MiniAct::Tick);
                    }
                    if ui.add_enabled(!self.hotkey_ticks.is_empty(), egui::Button::new("Untick")).on_hover_text("Ctrl+Alt+Backspace").clicked() {
                        act = Some(MiniAct::Untick);
                    }
                    let web = guide.kind != Kind::Video && guide.kind != Kind::Sheet && !links::is_local(&guide.url);
                    if web && !s.anchor.is_empty() && ui.button("Find in guide").clicked() {
                        let page = if s.url.is_empty() { &guide.url } else { &s.url };
                        act = Some(MiniAct::Find(links::with_text(page, &s.anchor)));
                    }
                    if ui.button("JAGT").on_hover_text("Bring the main window back").clicked() {
                        act = Some(MiniAct::Focus);
                    }
                });
            }
            None => {
                ui.label(RichText::new("All done!").font(bold(18.0)).color(GREEN));
            }
        }
        match act {
            Some(MiniAct::Tick) => {
                self.tick_next();
            }
            Some(MiniAct::Untick) => self.untick_last(),
            Some(MiniAct::Find(url)) => {
                let _ = open::that_detached(url);
            }
            Some(MiniAct::Focus) => {
                ui.ctx().send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
                ui.ctx().send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
            None => {}
        }
    }

    /// The small always-on-top window: what to do next, while you play.
    fn mini_window(&mut self, ctx: &egui::Context) {
        if !self.mini_open {
            return;
        }
        let builder = egui::ViewportBuilder::default()
            .with_title("JAGT \u{2014} next")
            .with_inner_size([400.0, 180.0])
            .with_min_inner_size([260.0, 120.0])
            .with_always_on_top()
            .with_resizable(true);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("jagt-mini"), builder, |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                // no separate windows (tests, some Linux setups): an ordinary egui window instead
                let mut open = true;
                egui::Window::new(RichText::new("Next").font(bold(16.0))).id(egui::Id::new("mini-embedded")).open(&mut open).default_width(380.0).show(ctx, |ui| self.mini_contents(ui));
                if !open {
                    self.mini_open = false;
                }
                return;
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(INK).stroke(Stroke::new(2.0f32, YELLOW)).inner_margin(egui::Margin::same(8))).show(ctx, |ui| self.mini_contents(ui));
            if ctx.input(|i| i.viewport().close_requested()) {
                self.mini_open = false;
            }
        });
    }

    /// Checklist, with the guide's text beside it (or below, when the window is narrow).
    fn checklist_area(&mut self, ui: &mut Ui, gid: u64, lone: bool) -> Option<WinButton> {
        if !self.reader_open {
            return window_ctl(ui, "Checklist", lone, |ui| self.checklist_view(ui, gid)).1;
        }
        let avail = ui.available_rect_before_wrap();
        let gap = 10.0;
        let (a, b) = if avail.width() >= 700.0 {
            let w = ((avail.width() - gap) * 0.5).floor();
            (Rect::from_min_size(avail.min, Vec2::new(w, avail.height())), Rect::from_min_max(Pos2::new(avail.left() + w + gap, avail.top()), avail.max))
        } else {
            let h = ((avail.height() - gap) * 0.55).floor();
            (Rect::from_min_size(avail.min, Vec2::new(avail.width(), h)), Rect::from_min_max(Pos2::new(avail.left(), avail.top() + h + gap), avail.max))
        };
        let mut button = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(a), |ui| {
            button = window_ctl(ui, "Checklist", lone, |ui| self.checklist_view(ui, gid)).1;
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(b), |ui| {
            window(ui, "Guide text", |ui| self.reader_view(ui, gid));
        });
        ui.allocate_rect(avail, Sense::hover());
        button
    }

    /// The guide's own text, scrolled to the item you're on.
    fn reader_view(&mut self, ui: &mut Ui, gid: u64) {
        let Some(game) = self.lib.games.iter().find(|g| g.id == gid) else { return };
        let Some(aid) = self.active_list.filter(|id| game.guides.iter().any(|g| g.id == *id)) else {
            ui.label(RichText::new("Pick a guide's checklist first.").color(DIM));
            return;
        };
        let guide = game.guides.iter().find(|g| g.id == aid).unwrap();
        let today = game.today();
        let (url, kind) = (guide.url.clone(), guide.kind);
        // the item to follow: the keyboard highlight, else the next one
        let item = self.list_cursor.filter(|(g, _)| *g == aid).map(|(_, i)| i).or_else(|| guide.next_index(today));
        let item_info = item.and_then(|i| guide.steps.get(i).map(|s| (i, s.section.clone(), s.text.clone(), s.anchor.clone())));

        if self.reader.as_ref().is_none_or(|r| r.guide != aid) {
            let text = model::load_page(&self.pages_dir, aid).or_else(|| {
                // a guide that is a text / web page file on this PC is read straight from it
                let path = links::local_path(&url)?;
                let ext = path.extension()?.to_str()?.to_ascii_lowercase();
                let raw = String::from_utf8_lossy(&std::fs::read(&path).ok()?).into_owned();
                match ext.as_str() {
                    "html" | "htm" => Some(online::html_to_text(&raw)),
                    "txt" | "md" | "csv" => Some(raw),
                    _ => None,
                }
            });
            self.reader = text.map(|t| Reader::new(aid, &t));
        }
        let web = kind != Kind::Video && kind != Kind::Sheet && !links::is_local(&url);
        if self.reader.is_none() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No text for this guide yet").font(bold(18.0)).color(TEAL));
                ui.label(RichText::new("Fetch the page, or paste its text with Paste checklist (the text is kept for reading here).").small().color(DIM));
                ui.add_space(6.0);
                let fetching = self.reader_fetching == Some(aid);
                if web && ui.add_enabled(!fetching, egui::Button::new(if fetching { "Fetching\u{2026}" } else { "Fetch the page text" })).clicked() {
                    if self.online_on {
                        self.reader_fetching = Some(aid);
                        self.online.send(Job::PageText { guide: aid, url: url.clone() });
                    } else {
                        self.set_status("Turn Online on to fetch the page");
                    }
                }
                if ui.button("Paste the text\u{2026}").clicked() {
                    self.bulk = Some((aid, String::new()));
                }
            });
            ui.allocate_space(ui.available_size());
            return;
        }
        let r = self.reader.as_mut().unwrap();

        // follow the item
        let key = item_info.as_ref().map(|(i, _, t, _)| format!("{i}:{t}")).unwrap_or_default();
        if key != r.target_key {
            r.target_key = key;
            r.target = item_info.as_ref().and_then(|(_, sec, text, anchor)| find_line(&r.lines, sec, text, anchor));
            if r.target.is_some() {
                r.jump = r.target;
            }
        }
        // ---- toolbar
        let mut refetch = false;
        ui.horizontal_wrapped(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut self.reader_find).id_salt(("reader-find", aid)).hint_text("Find in the text\u{2026}").desired_width(150.0));
            let q = self.reader_find.trim().to_lowercase();
            if q != r.found_for {
                r.found_for = q.clone();
                r.found = if q.chars().count() < 2 { Vec::new() } else { r.lines.iter().enumerate().filter(|(_, l)| l.to_lowercase().contains(&q)).map(|(i, _)| i).collect() };
                r.found_at = 0;
                r.jump = r.found.first().copied();
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if !r.found.is_empty() {
                ui.label(RichText::new(format!("{}/{}", r.found_at + 1, r.found.len())).small().color(TEAL));
                if ui.small_button("Next").clicked() || enter {
                    r.found_at = (r.found_at + 1) % r.found.len();
                    r.jump = Some(r.found[r.found_at]);
                    if enter {
                        resp.request_focus();
                    }
                }
            } else if q.chars().count() >= 2 {
                ui.label(RichText::new("not found").small().color(DIM));
            }
            if ui.add_enabled(r.target.is_some(), egui::Button::new("My place")).on_hover_text("Back to the item you're on").clicked() {
                r.jump = r.target;
            }
            if web && ui.small_button("Fetch again").on_hover_text("Download the page's text again").clicked() {
                refetch = true;
            }
            if item_info.is_some() && r.target.is_none() {
                ui.label(RichText::new("(couldn't find your item in the text)").small().color(DIM));
            }
        });
        ui.add_space(2.0);

        // ---- the text: only the lines on screen are laid out
        let width = (ui.available_width() - 14.0).max(80.0);
        if (r.width - width).abs() > 1.0 {
            r.width = width;
            r.heights = r.lines.iter().map(|l| estimate_line(l, width)).collect();
        }
        let mut tops = Vec::with_capacity(r.lines.len() + 1);
        let mut y = 0.0;
        for h in &r.heights {
            tops.push(y);
            y += h;
        }
        let total = y;
        let mut area = egui::ScrollArea::vertical().id_salt(("reader", aid)).auto_shrink([false, false]);
        if let Some(t) = r.jump.take() {
            area = area.vertical_scroll_offset((tops.get(t).copied().unwrap_or(0.0) - 60.0).max(0.0));
        }
        let mut changed = false;
        let found: HashSet<usize> = r.found.iter().copied().collect();
        let current_find = r.found.get(r.found_at).copied();
        area.show_viewport(ui, |ui, vp| {
            let origin = ui.max_rect().min;
            ui.set_min_height(total);
            ui.set_min_width(width);
            let first = tops.partition_point(|&t| t <= vp.min.y).saturating_sub(1);
            for i in first..r.lines.len() {
                if tops[i] > vp.max.y {
                    break;
                }
                let line = &r.lines[i];
                let rect = Rect::from_min_size(origin + Vec2::new(0.0, tops[i]), Vec2::new(width, r.heights[i]));
                if Some(i) == r.target {
                    ui.painter().rect_filled(rect, 2.0, MAGENTA.gamma_multiply(0.22));
                    ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), 0.0, MAGENTA);
                } else if Some(i) == current_find {
                    ui.painter().rect_filled(rect, 2.0, YELLOW.gamma_multiply(0.25));
                } else if found.contains(&i) {
                    ui.painter().rect_filled(rect, 2.0, YELLOW.gamma_multiply(0.1));
                }
                if line.trim().is_empty() {
                    continue;
                }
                let text = match line.strip_prefix("## ") {
                    Some(h) => RichText::new(h).font(bold(18.0)).color(YELLOW),
                    None => RichText::new(line).color(WHITE),
                };
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.min + Vec2::new(6.0, 2.0), Vec2::new(width - 8.0, 10_000.0))).layout(Layout::top_down(Align::Min)));
                child.add(egui::Label::new(text).wrap());
                let h = child.min_rect().height() + 4.0;
                if (h - r.heights[i]).abs() > 0.5 {
                    r.heights[i] = h;
                    changed = true;
                }
            }
        });
        if changed {
            ui.ctx().request_repaint();
        }
        if refetch {
            if self.online_on {
                self.reader_fetching = Some(aid);
                self.online.send(Job::PageText { guide: aid, url });
                self.set_status("Fetching the page\u{2026}");
            } else {
                self.set_status("Turn Online on to fetch the page");
            }
        }
    }

    /// "New run" (New Game+): which ticks to keep.
    fn run_window(&mut self, ctx: &egui::Context) {
        let Some((gid, name, keep, keep_ach)) = &mut self.run_window else { return };
        let gid = *gid;
        let Some(game) = self.lib.games.iter().find(|g| g.id == gid) else {
            self.run_window = None;
            return;
        };
        let mut open = true;
        let mut start = false;
        egui::Window::new(RichText::new("Start a new run").font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("\"{}\" is kept with all its ticks: switch back to it from the Run menu any time.", game.run_label())).color(DIM));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").color(TEAL));
                    ui.add(egui::TextEdit::singleline(name).desired_width(220.0));
                });
                ui.checkbox(keep_ach, "Keep achievements and trophies ticked (you've earned them)");
                ui.add_space(4.0);
                ui.label(RichText::new("Keep these guides ticked as they are:").color(TEAL));
                for g in game.guides.iter().filter(|g| !g.steps.is_empty()) {
                    let mut on = keep.contains(&g.id);
                    let done = g.steps.iter().filter(|s| s.done).count();
                    if ui.checkbox(&mut on, format!("{}  ({done}/{})", g.title, g.steps.len())).changed() {
                        if on {
                            keep.insert(g.id);
                        } else {
                            keep.remove(&g.id);
                        }
                    }
                }
                ui.label(RichText::new("Everything else starts unticked.").small().color(DIM));
                ui.add_space(6.0);
                if ui.button(RichText::new("Start the new run").font(bold(16.0))).clicked() {
                    start = true;
                }
            });
        if start {
            let (name, keep, keep_ach) = (name.clone(), keep.iter().copied().collect::<Vec<u64>>(), *keep_ach);
            if let Some(g) = self.lib.game_mut(gid) {
                g.new_run(&name, &keep, keep_ach, model::now());
                let label = g.run_label();
                self.set_status(format!("Started {label}"));
            }
            self.revealed.clear();
            self.run_window = None;
        } else if !open {
            self.run_window = None;
        }
    }

    /// Editing an item's note.
    fn note_window(&mut self, ctx: &egui::Context) {
        let Some((lid, i, text)) = &mut self.note_edit else { return };
        let (lid, i) = (*lid, *i);
        let Some(step) = self.lib.games.iter().flat_map(|g| &g.guides).find(|g| g.id == lid).and_then(|g| g.steps.get(i)) else {
            self.note_edit = None;
            return;
        };
        let mut open = true;
        let mut save = false;
        egui::Window::new(RichText::new("Note").font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .default_width(420.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.label(RichText::new(&step.text).font(bold(16.0)).color(WHITE));
                let r = ui.add(egui::TextEdit::multiline(text).id_salt("note-edit").desired_rows(5).desired_width(f32::INFINITY).hint_text("e.g. Came back for this after chapter 5. Needs the grappling hook."));
                if !r.has_focus() && text.is_empty() {
                    r.request_focus();
                }
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Save").font(bold(16.0))).clicked() {
                        save = true;
                    }
                    ui.label(RichText::new("Shows under the item, and Find searches it.").small().color(DIM));
                });
            });
        if save {
            let t = text.trim().to_string();
            if let Some(s) = self.lib.games.iter_mut().flat_map(|g| &mut g.guides).find(|g| g.id == lid).and_then(|g| g.steps.get_mut(i)) {
                s.note = t;
            }
            self.note_edit = None;
        } else if !open {
            self.note_edit = None;
        }
    }

    fn tick_list_window(&mut self, ctx: &egui::Context) {
        let Some((gid, text, mode)) = &mut self.tick_list else { return };
        let guide_id = *gid;
        let Some(guide) = self.lib.games.iter().flat_map(|g| &g.guides).find(|g| g.id == guide_id) else {
            self.tick_list = None;
            return;
        };
        let matched = match_names(&guide.steps, text);
        let mut open = true;
        let mut apply = false;
        egui::Window::new(RichText::new("Tick from a list").font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("Checklist: {}", guide.title)).color(TEAL));
                ui.label(
                    RichText::new("Paste names, one per line. A whole trophy page from PSNProfiles or the PS5 is fine: descriptions, percentages and rarity lines simply don't match anything.")
                        .small()
                        .color(DIM),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.radio_value(mode, TickMode::Named, "Tick the items named");
                    ui.radio_value(mode, TickMode::AllExcept, "Tick everything except them").on_hover_text("For a list of what you still need, like unearned trophies");
                    ui.radio_value(mode, TickMode::Untick, "Untick the items named");
                });
                egui::ScrollArea::vertical().id_salt("tick-list-text").max_height(220.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(text).desired_rows(9).desired_width(f32::INFINITY).hint_text("The Traveller\nClosing the Book\nSummon Mask Time\n..."));
                });
                let n = matched.len();
                let total = guide.steps.len();
                let effect = match mode {
                    TickMode::Named => format!("{n} items will be ticked."),
                    TickMode::AllExcept => format!("{} items will be ticked; the {n} named stay unticked.", total - n),
                    TickMode::Untick => format!("{n} items will be unticked."),
                };
                ui.label(RichText::new(format!("Matched {n} of {total} items. {effect}")).color(if n > 0 { YELLOW } else { DIM }));
                if n > 0 {
                    egui::ScrollArea::vertical().id_salt("tick-list-matches").max_height(120.0).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for &i in &matched {
                                tag(ui, &guide.steps[i].text, TEAL);
                            }
                        });
                    });
                }
                if ui.add_enabled(n > 0, egui::Button::new(RichText::new("Apply").font(bold(16.0)))).clicked() {
                    apply = true;
                }
            });
        if apply {
            let mode = *mode;
            let today = self.lib.games.iter().find(|g| g.guides.iter().any(|x| x.id == guide_id)).and_then(|g| g.today());
            if let Some(g) = self.lib.games.iter_mut().flat_map(|g| g.guides.iter_mut()).find(|g| g.id == guide_id) {
                let set: HashSet<usize> = matched.iter().copied().collect();
                for (i, s) in g.steps.iter_mut().enumerate() {
                    let named = set.contains(&i);
                    match mode {
                        TickMode::Named if named => s.done = true,
                        TickMode::AllExcept => s.done = !named,
                        TickMode::Untick if named => s.done = false,
                        _ => {}
                    }
                }
                g.follow_checklist(today);
            }
            self.tick_list = None;
            self.set_status("Checklist updated (Undo reverts it)");
        } else if !open {
            self.tick_list = None;
        }
    }

    fn close_bulk(&mut self) {
        self.bulk = None;
        self.bulk_cache = None;
        self.bulk_error = None;
        self.bulk_fetching = false;
        self.bulk_headings = false;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new(RichText::new("Settings").font(bold(18.0)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                let heading = |ui: &mut Ui, text: &str| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(text).font(bold(16.0)).color(TEAL));
                    ui.separator();
                };
                heading(ui, "Look");
                ui.checkbox(&mut self.animate, "Animated background (drifting stars)")
                    .on_hover_text("Off: a still background, and JAGT only redraws when something changes.");
                if ui.checkbox(&mut self.glow, "Neon glow (HDR-style bloom)").changed() {
                    set_glow(self.glow);
                }

                heading(ui, "Online");
                if ui.checkbox(&mut self.online_on, "Look up guide titles, site icons and box art").changed() && self.online_on {
                    self.favicons.retain(|_, f| !matches!(f, Fav::Missing));
                    self.covers.retain(|_, f| !matches!(f, Fav::Missing));
                }
                ui.label(RichText::new("Titles come from the page itself (YouTube via oEmbed). Icons come from Google's favicon service and are cached.").small().color(DIM));
                if ui.button("Clear icon and cover cache").clicked() {
                    online::clear_cache();
                    let _ = std::fs::remove_dir_all(online::cover_dir());
                    self.covers.clear();
                    self.favicons.clear();
                    self.set_status("Icon cache cleared");
                }

                if ui
                    .add_enabled(self.link_checks == 0, egui::Button::new(if self.link_checks > 0 { format!("Checking links\u{2026} {} to go", self.link_checks) } else { "Check every guide link".to_string() }))
                    .on_hover_text("Finds guide pages that have gone (404), and offers the Wayback Machine's copy")
                    .clicked()
                {
                    self.check_links(None);
                }

                heading(ui, "While you play");
                ui.checkbox(&mut self.mini_open, "Mini window: your next item, always on top");
                for (_, keys, what) in hotkeys::KEYS {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(keys).color(TEAL));
                        ui.label(RichText::new(what).small().color(DIM));
                        match &self.hotkeys {
                            Some(h) if h.taken.contains(&keys) => {
                                ui.label(RichText::new("(another program uses it)").small().color(MAGENTA));
                            }
                            None => {
                                ui.label(RichText::new("(not available)").small().color(DIM));
                            }
                            _ => {}
                        }
                    });
                }
                ui.label(RichText::new("These work even while the game has the focus. Continue starts the play timer; closing JAGT stops it.").small().color(DIM));

                heading(ui, "Accounts (achievement sync)");
                ui.label(RichText::new("Used by Sync achievements on each game. Stored only in this PC's settings file.").small().color(DIM));
                egui::Grid::new("accounts").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label(RichText::new("Steam API key").color(TEAL));
                    ui.add(egui::TextEdit::singleline(&mut self.steam_key).password(true).desired_width(240.0).hint_text("from steamcommunity.com/dev/apikey"));
                    ui.end_row();
                    ui.label(RichText::new("Steam profile").color(TEAL));
                    ui.add(egui::TextEdit::singleline(&mut self.steam_profile).desired_width(240.0).hint_text("profile link or 17-digit SteamID"));
                    ui.end_row();
                    ui.label(RichText::new("OpenXBL key").color(TEAL));
                    ui.add(egui::TextEdit::singleline(&mut self.xbox_key).password(true).desired_width(240.0).hint_text("from xbl.io after signing in"));
                    ui.end_row();
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(!self.steam_key.trim().is_empty() && !self.steam_profile.trim().is_empty(), egui::Button::new("Check Steam")).clicked() {
                        if self.online_on {
                            self.account_note = "Checking Steam\u{2026}".into();
                            self.online.send(Job::CheckAccount { source: Source::Steam, key: self.steam_key.trim().to_string(), profile: self.steam_profile.trim().to_string() });
                        } else {
                            self.account_note = "Turn Online on first.".into();
                        }
                    }
                    if ui.add_enabled(!self.xbox_key.trim().is_empty(), egui::Button::new("Check Xbox")).clicked() {
                        if self.online_on {
                            self.account_note = "Checking Xbox\u{2026}".into();
                            self.online.send(Job::CheckAccount { source: Source::Xbox, key: self.xbox_key.trim().to_string(), profile: String::new() });
                        } else {
                            self.account_note = "Turn Online on first.".into();
                        }
                    }
                    ui.hyperlink_to(RichText::new("Get a Steam key").small(), "https://steamcommunity.com/dev/apikey");
                    ui.hyperlink_to(RichText::new("Get an OpenXBL key").small(), "https://xbl.io");
                });
                let mut linked = Vec::new();
                if !self.steam_name.is_empty() {
                    linked.push(format!("Steam: {}", self.steam_name));
                }
                if !self.xbox_tag.is_empty() {
                    linked.push(format!("Xbox: {}", self.xbox_tag));
                }
                if !linked.is_empty() {
                    ui.label(RichText::new(format!("Linked \u{2014} {}", linked.join(" \u{b7} "))).small().color(GREEN));
                }
                if !self.account_note.is_empty() {
                    ui.label(RichText::new(&self.account_note).small().color(YELLOW));
                }
                ui.label(RichText::new("PlayStation: no public trophy API. Paste your trophy list with Checklist \u{25b8} \u{2026} \u{25b8} Tick from a list.").small().color(DIM));

                heading(ui, "Your data");
                ui.label(RichText::new(self.lib_path.display().to_string()).small().color(YELLOW));
                ui.label(
                    RichText::new("Saved automatically. The previous version is kept as library.bak.json. Put a jagt-data folder next to the .exe for portable mode.")
                        .small()
                        .color(DIM),
                );
                let backups = model::list_backups(&self.lib_path);
                ui.label(RichText::new(format!("Daily backups (last {} days)", model::BACKUP_DAYS)).color(TEAL));
                if backups.is_empty() {
                    ui.label(RichText::new("The first one is made with today's first save.").small().color(DIM));
                }
                let now = self.time();
                let mut restore = None;
                egui::ScrollArea::vertical().id_salt("backups").max_height(120.0).show(ui, |ui| {
                    for (path, date) in &backups {
                        ui.horizontal(|ui| {
                            let games = model::read_file(path).map(|l| l.games.len()).unwrap_or(0);
                            ui.label(RichText::new(format!("{date}  ·  {games} games")).small().color(YELLOW));
                            if armed_button(ui, ("restore", date.as_str()), "Restore", now) {
                                restore = Some((path.clone(), date.clone()));
                            }
                        });
                    }
                });
                if let Some((path, date)) = restore {
                    match model::read_file(&path) {
                        Ok(lib) => {
                            self.lib = lib;
                            if self.selected.is_some_and(|id| !self.lib.games.iter().any(|g| g.id == id)) {
                                self.selected = self.sorted_ids().first().copied();
                            }
                            self.set_status(format!("Restored the {date} backup (Undo brings back what you had)"));
                        }
                        Err(e) => self.set_status_for(format!("Couldn't read that backup: {e}"), 6.0),
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Open data folder").clicked() {
                        if let Some(dir) = self.lib_path.parent() {
                            let _ = open::that_detached(dir);
                        }
                    }
                    if ui.button("Export…").clicked() {
                        self.flush();
                        if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).set_file_name("jagt-library.json").save_file() {
                            match model::save(&p, &self.lib) {
                                Ok(()) => self.set_status("Exported"),
                                Err(e) => self.set_status_for(format!("Export failed: {e}"), 6.0),
                            }
                        }
                    }
                    if ui.button("Import…").on_hover_text("Adds the games from a JAGT export to this library").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                            match model::read_file(&p) {
                                Ok(other) => {
                                    let n = self.lib.merge(other);
                                    self.set_status(format!("Imported {n} games"));
                                }
                                Err(e) => self.set_status_for(format!("Import failed: {e}"), 6.0),
                            }
                        }
                    }
                });

                heading(ui, "Shortcuts");
                for (k, v) in [
                    ("Ctrl+V", "add the copied link as a guide (outside text boxes)"),
                    ("Enter", "continue: reopen the last guide at your spot"),
                    ("Ctrl+N", "add a game"),
                    ("Right-click", "a game to change its status; a log entry to delete it"),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(k).color(TEAL));
                        ui.label(RichText::new(v).small().color(DIM));
                    });
                }
            });
        self.show_settings = open;
    }

    /// Restores the saved window size/maximized state on the first frame, then keeps track of
    /// it. Minimized frames are ignored so a minimized close doesn't save a useless size.
    fn track_window(&mut self, ctx: &egui::Context) {
        if let Some((size, maximized)) = self.restore_window {
            let monitor = ctx.input(|i| i.viewport().monitor_size);
            if monitor.is_none() && self.restore_wait < 10 {
                self.restore_wait += 1;
                return;
            }
            self.restore_window = None;
            let size = size
                .map(|[w, h]| Vec2::new(w, h))
                .or_else(|| monitor.map(|m| m * 0.8))
                .map(|s| s.max(Vec2::new(900.0, 560.0)));
            if let Some(size) = size {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                if let Some(monitor) = monitor {
                    let pos = ((monitor - size) * 0.5).max(Vec2::ZERO);
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.to_pos2()));
                }
            }
            if maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            return;
        }
        let (maximized, minimized, inner) = ctx.input(|i| {
            let v = i.viewport();
            (v.maximized.unwrap_or(false), v.minimized.unwrap_or(false), v.inner_rect)
        });
        if minimized {
            return;
        }
        self.maximized = maximized;
        if let Some(r) = inner.filter(|r| !maximized && r.width() >= 400.0 && r.height() >= 300.0) {
            self.window_size = Some(r.size());
        }
    }
}

impl App {
    /// Draws one frame. Separate from eframe's `update` so tests can drive the real UI.
    fn frame(&mut self, ctx: &egui::Context) {
        self.track_window(ctx);
        let now = self.time();
        // Moving the mouse can't change the library, so those frames skip the (whole-library)
        // unsaved-changes check.
        let had_input = ctx.input(|i| {
            i.pointer.any_down()
                || i.pointer.any_released()
                || i.events.iter().any(|e| !matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerGone | egui::Event::WindowFocused(_)))
        });
        let mut pumped = self.pump(ctx);
        let pressed = self.hotkeys.as_ref().map(|h| h.poll()).unwrap_or_default();
        pumped |= !pressed.is_empty();
        for a in pressed {
            match a {
                HotAction::TickNext => {
                    if self.tick_next().is_none() {
                        self.set_status("Nothing to tick: pick a game with a checklist");
                    }
                }
                HotAction::UndoTick => self.untick_last(),
                HotAction::ToggleMini => self.mini_open = !self.mini_open,
            }
        }
        self.keyboard(ctx);
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.add_paths(dropped);
        }

        let (focused, minimized) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().minimized.unwrap_or(false)));
        let animating = self.animate && focused && !minimized;
        // the stars only drift while they're animating; otherwise they hold still
        let star_time = if animating {
            now
        } else {
            ctx.data(|d| d.get_temp::<f64>(egui::Id::new("star-time"))).unwrap_or(now)
        };
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("star-time"), star_time));
        paint_night_sky(&ctx.layer_painter(egui::LayerId::background()), ctx.screen_rect(), star_time);

        egui::TopBottomPanel::top("header").frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 6))).show(ctx, |ui| self.header(ui));

        egui::TopBottomPanel::bottom("taskbar")
            .exact_height(40.0)
            .frame(egui::Frame::new().fill(MAGENTA).inner_margin(egui::Margin::symmetric(6, 4)))
            .show(ctx, |ui| {
                let r = ui.max_rect().expand2(Vec2::new(6.0, 4.0));
                ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(2.0f32, WHITE));
                self.taskbar(ui);
            });

        let pad = egui::Frame::new().inner_margin(egui::Margin::same(10));
        egui::SidePanel::left("games").resizable(true).default_width(300.0).width_range(220.0..=460.0).frame(pad).show(ctx, |ui| {
            contained(ui, |ui| window(ui, "Games", |ui| self.games_view(ui)));
        });

        let gid = self.selected.filter(|id| self.lib.games.iter().any(|g| g.id == *id));
        let has_guides = gid.is_some_and(|id| self.lib.games.iter().any(|g| g.id == id && !g.guides.is_empty()));
        // the view in effect: a game's checklist needs a guide; no game means the usual layout
        let view = match self.view {
            ViewMode::Checklist if !has_guides => ViewMode::All,
            v if gid.is_some() => v,
            _ => ViewMode::All,
        };
        // a title-bar button clicked this frame: (which window, which button)
        let mut win: Option<(ViewMode, WinButton)> = None;
        if let Some(gid) = gid.filter(|_| !self.explorer_open && !self.trophies_open && !self.missables_open && view == ViewMode::All) {
            egui::SidePanel::right("journal").resizable(true).default_width(330.0).width_range(260.0..=520.0).frame(pad).show(ctx, |ui| {
                contained(ui, |ui| {
                    if let (_, Some(b)) = window_ctl(ui, "Where I Left Off", false, |ui| self.resume_view(ui, gid)) {
                        win = Some((ViewMode::Notes, b));
                    }
                    ui.add_space(12.0);
                    if let (_, Some(b)) = window_ctl(ui, "Session Log", false, |ui| self.log_view(ui, gid)) {
                        win = Some((ViewMode::Notes, b));
                    }
                });
            });
        }

        egui::CentralPanel::default().frame(pad).show(ctx, |ui| match gid {
            _ if self.explorer_open => {
                window(ui, "Explorer", |ui| self.explorer_view(ui));
            }
            _ if self.trophies_open => {
                window(ui, "Trophies", |ui| self.trophies_view(ui));
            }
            _ if self.missables_open => {
                window(ui, "Missables", |ui| self.missables_view(ui));
            }
            Some(gid) if view != ViewMode::All => {
                // one window gets all the space
                let (_, b) = match view {
                    ViewMode::Game => window_ctl(ui, "Game", true, |ui| {
                        self.game_header(ui, gid);
                        ui.allocate_space(ui.available_size());
                    }),
                    ViewMode::Guides => window_ctl(ui, "Guides", true, |ui| self.guides_view(ui, gid, f32::INFINITY)),
                    ViewMode::Checklist => ((), self.checklist_area(ui, gid, true)),
                    _ => {
                        let (_, a) = window_ctl(ui, "Where I Left Off", true, |ui| self.resume_view(ui, gid));
                        ui.add_space(12.0);
                        let (_, b) = window_ctl(ui, "Session Log", true, |ui| self.log_view(ui, gid));
                        ((), a.or(b))
                    }
                };
                if let Some(b) = b {
                    win = Some((view, b));
                }
            }
            Some(gid) => {
                if let (_, Some(b)) = window_ctl(ui, "Game", false, |ui| self.game_header(ui, gid)) {
                    win = Some((ViewMode::Game, b));
                }
                ui.add_space(12.0);
                if has_guides {
                    let max_h = (ui.available_height() * 0.28).max(130.0);
                    if let (_, Some(b)) = window_ctl(ui, "Guides", false, |ui| self.guides_view(ui, gid, max_h)) {
                        win = Some((ViewMode::Guides, b));
                    }
                    ui.add_space(12.0);
                    if let Some(b) = self.checklist_area(ui, gid, false) {
                        win = Some((ViewMode::Checklist, b));
                    }
                } else if let (_, Some(b)) = window_ctl(ui, "Guides", false, |ui| self.guides_view(ui, gid, f32::INFINITY)) {
                    win = Some((ViewMode::Guides, b));
                }
            }
            None => {
                window(ui, "Welcome", |ui| {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Never lose your place in a guide again").font(bold(22.0)).color(TEAL));
                        ui.add_space(8.0);
                        for line in [
                            "1. Add a game.",
                            "2. Paste links to the guides you use: websites, spreadsheets, YouTube, wikis, trophy guides.",
                            "3. Note where you are in each one, and where you left off in the game.",
                            "4. Next time, press Continue to jump straight back in.",
                        ] {
                            ui.label(RichText::new(line).color(YELLOW));
                        }
                        ui.add_space(12.0);
                        if ui.button("+ Add your first game").clicked() {
                            self.add_game = Some(NewGame { title: String::new(), platform: String::new(), status: Status::Playing, focus: true });
                        }
                    });
                    ui.allocate_space(ui.available_size());
                });
            }
        });
        match win {
            // □ on a window: show only it; □ (restore) or × on a lone window: back to all
            Some((w, WinButton::Maximize)) => self.view = if view == ViewMode::All { w } else { ViewMode::All },
            Some((_, WinButton::Close)) => self.view = ViewMode::All,
            None => {}
        }

        if let Some((msg, until)) = &self.status {
            if now < *until {
                egui::Area::new("status".into()).anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -52.0)).show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(RichText::new(msg).color(YELLOW)));
                });
            } else {
                self.status = None;
            }
        }

        self.add_game_window(ctx);
        self.bulk_window(ctx);
        self.tick_list_window(ctx);
        self.search_all_window(ctx);
        self.sync_window(ctx);
        self.run_window(ctx);
        self.note_window(ctx);
        self.settings_window(ctx);
        self.mini_window(ctx);
        self.autosave(had_input || pumped);
        self.watch_file(now);

        // Redraw at ~15 fps only while the stars are drifting. Otherwise egui redraws on input, and a
        // 1 s tick keeps autosave and the library-file check going (and status messages expiring).
        let tick = if animating { 66 } else { 1000 };
        ctx.request_repaint_after(std::time::Duration::from_millis(tick));
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.flush();
        if !self.persist {
            return;
        }
        let saved = Saved {
            filter: self.filter,
            selected: self.selected,
            online: self.online_on,
            glow: self.glow,
            animate: self.animate,
            folder_view: self.folder_view,
            closed_folders: self.closed_folders.clone(),
            ex_layout: self.ex_layout,
            compact_rows: self.compact_rows,
            view: self.view,
            steam_key: self.steam_key.clone(),
            steam_profile: self.steam_profile.clone(),
            steam_id: self.steam_id.clone(),
            steam_name: self.steam_name.clone(),
            xbox_key: self.xbox_key.clone(),
            xbox_tag: self.xbox_tag.clone(),
            explorer_open: self.explorer_open,
            ex_path: self.ex_path.clone(),
            window_size: self.window_size.map(|s| [s.x, s.y]),
            maximized: self.maximized,
            mini_open: self.mini_open,
            tag_filter: self.tag_filter.clone(),
            reader_open: self.reader_open,
        };
        eframe::set_value(storage, "jagt", &saved);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // closing JAGT ends the play session
        let running: Vec<u64> = self.lib.games.iter().filter(|g| g.session_start.is_some()).map(|g| g.id).collect();
        for id in running {
            self.stop_playing(id);
        }
        self.flush();
    }

    /// Don't bring back last session's scroll positions: lists should open at the top.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30)
    }
}

// ---------------------------------------------------------------------------------------------
// Guide card

struct CardCtx<'a> {
    fav: Option<&'a TextureHandle>,
    expanded: bool,
    last_used: bool,
    first: bool,
    last: bool,
    time_buf: &'a mut String,
    /// This guide's checklist is the one on show.
    active: bool,
    /// Set when the player's place in the guide moved (marks the game as played).
    progressed: bool,
    toggle: bool,
}

fn guide_card(ui: &mut Ui, g: &mut Guide, c: &mut CardCtx) -> Vec<CardAct> {
    let mut acts = Vec::new();
    let border = if c.last_used { MAGENTA } else { INDIGO };
    // Reference links (no checklist, nothing to fill in) take one line until expanded.
    // Reference links, and guides whose checklist isn't the one on show, take one line until expanded.
    let compact = (g.steps.is_empty() || !c.active) && !matches!(g.kind, Kind::Video | Kind::Sheet) && !c.expanded;
    let resp = egui::Frame::new()
        .fill(CARD_BG)
        .stroke(Stroke::new(1.0f32, border))
        .corner_radius(2)
        .inner_margin(if compact { egui::Margin::symmetric(6, 3) } else { egui::Margin::same(6) })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // ---- title row
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                match c.fav {
                    Some(t) => {
                        ui.painter().image(t.id(), ir, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    None => icons::kind_badge(ui.painter(), ir, g.kind),
                }
                // expand arrow
                let (ar, aresp) = ui.allocate_exact_size(Vec2::new(14.0, 20.0), Sense::click());
                let col = if aresp.hovered() { TEAL } else { YELLOW };
                let k = ar.center();
                let tri = if c.expanded {
                    vec![k + Vec2::new(-4.0, -2.0), k + Vec2::new(4.0, -2.0), k + Vec2::new(0.0, 3.0)]
                } else {
                    vec![k + Vec2::new(-2.0, -4.0), k + Vec2::new(3.0, 0.0), k + Vec2::new(-2.0, 4.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(tri, col, Stroke::NONE));
                if aresp.clicked() {
                    c.toggle = true;
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button("…", |ui| {
                        if ui.add_enabled(!c.first, egui::Button::new("Move up")).clicked() {
                            acts.push(CardAct::Up);
                            ui.close_menu();
                        }
                        if ui.add_enabled(!c.last, egui::Button::new("Move down")).clicked() {
                            acts.push(CardAct::Down);
                            ui.close_menu();
                        }
                        if ui.button("Copy link").clicked() {
                            ui.ctx().copy_text(g.resume_url());
                            ui.close_menu();
                        }
                        if links::is_local(&g.url) {
                            if ui.button("Show in folder").clicked() {
                                acts.push(CardAct::ShowInFolder);
                                ui.close_menu();
                            }
                        } else {
                            if ui.button("Look up title again").clicked() {
                                acts.push(CardAct::Refetch);
                                ui.close_menu();
                            }
                            if ui.button("Check the page still exists").clicked() {
                                acts.push(CardAct::CheckLink);
                                ui.close_menu();
                            }
                            if ui.button("Open the archived copy").on_hover_text("The Wayback Machine's latest copy of this page").clicked() {
                                acts.push(CardAct::Archive);
                                ui.close_menu();
                            }
                        }
                        if ui.button("Paste checklist…").clicked() {
                            acts.push(CardAct::BulkSteps);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button(RichText::new("Remove guide").color(MAGENTA)).clicked() {
                            acts.push(CardAct::Remove);
                            ui.close_menu();
                        }
                    });
                    if g.dead {
                        if ui.button(RichText::new("Archived copy").color(YELLOW)).on_hover_text("The page is gone: open the Wayback Machine's copy").clicked() {
                            acts.push(CardAct::Archive);
                        }
                        tag(ui, "page gone", MAGENTA);
                    }
                    if compact && !g.steps.is_empty() {
                        let label = format!("Checklist {}/{}", g.steps.iter().filter(|s| s.done).count(), g.steps.len());
                        if ui.button(RichText::new(label).color(TEAL)).on_hover_text("Show this guide's checklist").clicked() {
                            acts.push(CardAct::ShowChecklist);
                        }
                    }
                    let open = ui
                        .add(egui::Button::new(RichText::new("Resume").font(bold(16.0)).color(if c.last_used { MAGENTA } else { YELLOW })))
                        .on_hover_text(format!("Open at your saved spot\n{}", g.resume_url()));
                    if open.clicked() {
                        acts.push(CardAct::Open);
                    }
                    if ui.checkbox(&mut g.done, "Done").changed() {
                        c.progressed = true;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let mut t = RichText::new(&g.title).font(bold(16.0)).color(if g.done { DIM } else { WHITE });
                        if g.done {
                            t = t.strikethrough();
                        }
                        let r = ui.add(egui::Label::new(t).truncate().sense(Sense::click())).on_hover_text(&g.url);
                        if r.clicked() {
                            c.toggle = true;
                        }
                    });
                });
            });

            if compact {
                return;
            }
            // ---- info row
            ui.horizontal(|ui| {
                ui.add_space(42.0);
                let place = if links::is_local(&g.url) { "file on this PC".to_string() } else { links::host(&g.url) };
                let mut info = vec![g.kind.label().to_string(), place];
                info.push(match g.last_opened {
                    Some(t) => format!("opened {}", model::ago(t)),
                    None => "not opened yet".into(),
                });
                ui.label(RichText::new(info.join(" · ")).small().color(DIM));
                if c.last_used {
                    ui.label(RichText::new("· continue here").small().color(MAGENTA));
                }
            });

            // ---- progress
            ui.horizontal(|ui| {
                ui.add_space(42.0);
                let w = (ui.available_width() - 170.0).max(60.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::hover());
                let bar = Rect::from_center_size(r.center(), Vec2::new(r.width() - 8.0, 6.0));
                let manual = g.steps.is_empty() && !g.done;
                if let Some(f) = rail(ui, bar, g.progress(), manual, ("rail", g.id)) {
                    g.percent = (f * 20.0).round() / 20.0;
                    c.progressed = true;
                }
                if g.steps.is_empty() {
                    ui.label(RichText::new(format!("{:.0}%", g.progress() * 100.0)).color(TEAL));
                } else {
                    let label = format!("Checklist {}/{}", g.steps.iter().filter(|s| s.done).count(), g.steps.len());
                    if ui.add(egui::Button::new(RichText::new(label).color(TEAL)).selected(c.active)).on_hover_text("Show this guide's checklist").clicked() {
                        acts.push(CardAct::ShowChecklist);
                    }
                }
            });

            // ---- where I am
            ui.horizontal(|ui| {
                ui.add_space(42.0);
                match g.kind {
                    Kind::Video => {
                        ui.label(RichText::new("Time").color(TEAL));
                        let r = ui
                            .add(egui::TextEdit::singleline(c.time_buf).id_salt(("time", g.id)).desired_width(64.0).hint_text("0:00"))
                            .on_hover_text("How far into the video you are: 1:02:03, 62:03 or 1h2m3s");
                        let editing = r.has_focus();
                        if r.changed() {
                            if let Some(t) = links::parse_hms(c.time_buf) {
                                g.video_secs = t;
                                c.progressed = true;
                            }
                        }
                        for (lbl, d) in [("-1m", -60i64), ("+1m", 60)] {
                            if ui.small_button(lbl).clicked() {
                                g.video_secs = (g.video_secs as i64 + d).max(0) as u32;
                                c.progressed = true;
                            }
                        }
                        if !editing {
                            let want = links::fmt_hms(g.video_secs);
                            if *c.time_buf != want && links::parse_hms(c.time_buf) != Some(g.video_secs) {
                                *c.time_buf = want;
                            }
                        }
                    }
                    Kind::Sheet => {
                        ui.label(RichText::new("Row").color(TEAL));
                        let r = ui.add(egui::DragValue::new(&mut g.sheet_row).range(0..=1_000_000)).on_hover_text("The spreadsheet row you're on. Google Sheets opens scrolled to it.");
                        if r.changed() {
                            c.progressed = true;
                        }
                    }
                    _ => {
                        ui.label(RichText::new("Jump to").color(TEAL));
                        let r = ui
                            .add(egui::TextEdit::singleline(&mut g.jump_text).id_salt(("jump", g.id)).desired_width(170.0).hint_text("a heading on the page"))
                            .on_hover_text("Copy a heading or phrase from the guide. Resume scrolls the page straight to it.");
                        if r.changed() {
                            c.progressed = true;
                        }
                    }
                }
                ui.label(RichText::new("Where").color(TEAL));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut g.position)
                        .id_salt(("pos", g.id))
                        .desired_width(ui.available_width())
                        .hint_text("e.g. Chapter 4, after the bridge"),
                );
                if r.changed() {
                    c.progressed = true;
                }
            });

            if !c.expanded {
                return;
            }
            // ---- details
            ui.add_space(4.0);
            ui.separator();
            egui::Grid::new(("details", g.id)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label(RichText::new("Title").color(TEAL));
                if ui.add(egui::TextEdit::singleline(&mut g.title).id_salt(("title", g.id)).desired_width(f32::INFINITY)).changed() {
                    g.auto_title = false;
                }
                ui.end_row();
                ui.label(RichText::new("Link").color(TEAL));
                ui.add(egui::TextEdit::singleline(&mut g.url).id_salt(("url", g.id)).desired_width(f32::INFINITY));
                ui.end_row();
                ui.label(RichText::new("Type").color(TEAL));
                egui::ComboBox::from_id_salt(("kind", g.id)).selected_text(g.kind.label()).show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        ui.selectable_value(&mut g.kind, k, k.label());
                    }
                });
                ui.end_row();
            });

            ui.add_space(4.0);
            ui.label(RichText::new("Notes").font(bold(16.0)).color(TEAL));
            ui.add(
                egui::TextEdit::multiline(&mut g.notes)
                    .id_salt(("notes", g.id))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Missables, tips, what this guide is good for…"),
            );
        });
    if c.last_used {
        glow_rect(ui.painter(), resp.response.rect, 2.0, MAGENTA, 4.0);
    }
    acts
}

/// A progress rail. When `interactive`, clicking/dragging returns the new fraction.
fn rail(ui: &mut Ui, rect: Rect, frac: f32, interactive: bool, id: impl Hash) -> Option<f32> {
    let sense = if interactive { Sense::click_and_drag() } else { Sense::hover() };
    let resp = ui.interact(rect.expand2(Vec2::new(4.0, 6.0)), ui.id().with(id), sense);
    let p = ui.painter();
    p.rect_filled(rect.expand(2.0), 2.0, INK);
    p.rect_stroke(rect.expand(2.0), 2.0, Stroke::new(1.0f32, INDIGO), StrokeKind::Outside);
    let mut f = frac.clamp(0.0, 1.0);
    if interactive {
        if let Some(pos) = resp.interact_pointer_pos() {
            f = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        }
    }
    let fill = if f >= 1.0 { Color32::from_rgb(0x8C, 0xF0, 0x8C) } else { YELLOW };
    let filled = Rect::from_min_max(rect.min, Pos2::new(rect.left() + rect.width() * f, rect.bottom()));
    p.rect_filled(filled, rect.height() * 0.5, fill);
    if interactive {
        let knob = if resp.hovered() || resp.dragged() { TEAL } else { WHITE };
        p.circle_filled(Pos2::new(filled.right(), rect.center().y), rect.height() * 0.9 + 1.0, knob);
        if resp.dragged() || resp.clicked() {
            return Some(f);
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Checklist pieces

fn mtime(p: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// A full-width area heading with its own count. Click to fold the area away.
fn section_header(ui: &mut Ui, title: &str, done: usize, total: usize, shut: bool) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
    let p = ui.painter();
    let complete = total > 0 && done == total;
    p.rect_filled(rect, 2.0, if resp.hovered() { INDIGO } else { PURPLE });
    p.rect_stroke(rect, 2.0, Stroke::new(1.5f32, if complete { Color32::from_rgb(0x8C, 0xF0, 0x8C) } else { YELLOW }), StrokeKind::Inside);
    let k = Pos2::new(rect.left() + 14.0, rect.center().y);
    let tri = if shut {
        vec![k + Vec2::new(-3.0, -5.0), k + Vec2::new(4.0, 0.0), k + Vec2::new(-3.0, 5.0)]
    } else {
        vec![k + Vec2::new(-5.0, -3.0), k + Vec2::new(5.0, -3.0), k + Vec2::new(0.0, 4.0)]
    };
    p.add(egui::Shape::convex_polygon(tri, YELLOW, Stroke::NONE));
    let right_w = 150.0;
    let title_rect = Rect::from_min_max(Pos2::new(rect.left() + 26.0, rect.top()), Pos2::new(rect.right() - right_w, rect.bottom()));
    p.with_clip_rect(title_rect).text(title_rect.left_center(), Align2::LEFT_CENTER, title, bold(18.0), if complete { Color32::from_rgb(0x8C, 0xF0, 0x8C) } else { YELLOW });
    let bar = Rect::from_min_size(Pos2::new(rect.right() - right_w + 8.0, rect.center().y - 3.0), Vec2::new(70.0, 6.0));
    p.rect_filled(bar, 2.0, INK);
    let f = if total == 0 { 0.0 } else { done as f32 / total as f32 };
    p.rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.left() + bar.width() * f, bar.bottom())), 2.0, if complete { Color32::from_rgb(0x8C, 0xF0, 0x8C) } else { TEAL });
    p.text(Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, format!("{done}/{total}"), pixel(16.0), TEAL);
    resp.on_hover_text(if shut { "Show this area" } else { "Fold this area away" })
}

/// A chunky pixel checkbox.
fn big_checkbox(ui: &mut Ui, checked: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let p = ui.painter();
    let border = if resp.hovered() { TEAL } else if checked { Color32::from_rgb(0x8C, 0xF0, 0x8C) } else { YELLOW };
    p.rect_filled(rect, 3.0, if checked { Color32::from_rgb(0x2E, 0x6B, 0x3A) } else { INK });
    p.rect_stroke(rect, 3.0, Stroke::new(2.0f32, border), StrokeKind::Inside);
    if checked {
        let c = rect.center();
        let st = Stroke::new(3.0f32, Color32::from_rgb(0x8C, 0xF0, 0x8C));
        p.line_segment([c + Vec2::new(-6.0, 0.0), c + Vec2::new(-2.0, 5.0)], st);
        p.line_segment([c + Vec2::new(-2.0, 5.0), c + Vec2::new(7.0, -6.0)], st);
    }
    resp
}

/// Guide files at `p` (a file, or a folder searched recursively, in name order), up to 500.
fn collect_guide_files(p: &std::path::Path, out: &mut Vec<PathBuf>, depth: usize) {
    if out.len() >= 500 || depth > 8 {
        return;
    }
    if p.is_dir() {
        let Ok(rd) = std::fs::read_dir(p) else { return };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort_by_key(|e| e.to_string_lossy().to_lowercase());
        for e in entries {
            collect_guide_files(&e, out, depth + 1);
        }
    } else if links::is_guide_file(p) {
        out.push(p.to_path_buf());
    }
}

/// Opens the file manager with the file selected (Windows), or its folder.
fn show_in_folder(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut arg = std::ffi::OsString::from("/select,\"");
        arg.push(path.as_os_str());
        arg.push("\"");
        std::process::Command::new("explorer").raw_arg(arg).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        open::that_detached(path.parent().unwrap_or(path))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
enum ExplorerLayout {
    #[default]
    Icons,
    Details,
}

#[derive(Clone, PartialEq, Debug)]
enum ExplorerItem {
    Folder(String),
    Game(u64),
}

/// A game as the explorer shows it (owned, so drawing doesn't hold on to the library).
struct GameTile {
    id: u64,
    title: String,
    platform: String,
    status: Status,
    folder: Option<Folder>,
    progress: f32,
    last_played: Option<i64>,
    host: String,
}

const TILE: Vec2 = Vec2::new(128.0, 150.0);

/// What's inside an explorer folder: its subfolders (name, games under them) and its games.
/// `[]` is the top (PC, Console...), `["Console"]` a folder, `["Console", "PS5"]` a platform.
/// `None` if the path doesn't exist (any more).
fn explorer_children(games: &[&Game], path: &[String]) -> Option<(Vec<(String, Vec<u64>)>, Vec<u64>)> {
    match path {
        [] => {
            let folders = Folder::ALL
                .iter()
                .map(|f| (f.label().to_string(), games.iter().filter(|g| g.folder() == *f).map(|g| g.id).collect::<Vec<_>>()))
                .filter(|(_, ids)| !ids.is_empty())
                .collect();
            Some((folders, Vec::new()))
        }
        [top, rest @ ..] => {
            let f = Folder::ALL.into_iter().find(|f| f.label() == top)?;
            let inside: Vec<&&Game> = games.iter().filter(|g| g.folder() == f).collect();
            let mut platforms: Vec<String> = Vec::new();
            for g in &inside {
                let name = platform_name(&g.platform);
                if !platforms.iter().any(|p| p.eq_ignore_ascii_case(&name)) {
                    platforms.push(name);
                }
            }
            platforms.sort_by_key(|p| p.to_lowercase());
            match rest {
                [] if platforms.len() > 1 => {
                    let folders = platforms
                        .iter()
                        .map(|p| (p.clone(), inside.iter().filter(|g| platform_name(&g.platform).eq_ignore_ascii_case(p)).map(|g| g.id).collect()))
                        .collect();
                    Some((folders, Vec::new()))
                }
                [] => Some((Vec::new(), inside.iter().map(|g| g.id).collect())),
                [pl] if platforms.len() > 1 => {
                    let ids: Vec<u64> = inside.iter().filter(|g| platform_name(&g.platform).eq_ignore_ascii_case(pl)).map(|g| g.id).collect();
                    (!ids.is_empty()).then_some((Vec::new(), ids))
                }
                _ => None,
            }
        }
    }
}

/// The colour of a platform's game cases: PlayStation blue, Nintendo red, Xbox green...
fn platform_color(platform: &str) -> Color32 {
    let p = platform.to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| p.contains(x));
    if has(&["playstation", "psp", "vita"]) || p.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with("ps")) {
        Color32::from_rgb(0x2F, 0x5B, 0xD8)
    } else if has(&["switch", "nintendo", "3ds", "wii", "game boy", "gameboy", "gba", "gamecube", "n64", "snes", "nes"]) || p.split(|c: char| !c.is_alphanumeric()).any(|w| w == "ds") {
        Color32::from_rgb(0xE6, 0x2A, 0x2A)
    } else if has(&["xbox"]) {
        Color32::from_rgb(0x2E, 0xA0, 0x43)
    } else if has(&["sega", "dreamcast", "genesis", "saturn", "mega drive"]) {
        Color32::from_rgb(0x1E, 0x88, 0xE5)
    } else {
        match Folder::of_platform(platform) {
            Folder::Pc => Color32::from_rgb(0x8A, 0x8F, 0xA8),
            Folder::Mobile => TEAL,
            Folder::Linux => Color32::from_rgb(0xF0, 0x8A, 0x24),
            _ => DIM,
        }
    }
}

/// Back / forward / up button.
fn tool_arrow(ui: &mut Ui, dir: Vec2, enabled: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 26.0), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(rect, 2.0, INDIGO);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0f32, TEAL), StrokeKind::Inside);
    }
    icons::arrow(p, rect, dir, if !enabled { DIM.gamma_multiply(0.5) } else if resp.hovered() { TEAL } else { YELLOW });
    resp.on_hover_text(tip)
}

/// One part of the breadcrumb path.
fn crumb(ui: &mut Ui, text: &str, current: bool) -> egui::Response {
    let t = RichText::new(text).color(if current { YELLOW } else { TEAL });
    ui.add(egui::Label::new(if current { t.font(bold(16.0)) } else { t }).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Tile background: highlighted when selected, lit on hover (like Explorer).
fn tile_bg(p: &egui::Painter, rect: Rect, selected: bool, hovered: bool) {
    if selected {
        p.rect_filled(rect, 3.0, INDIGO);
        p.rect_stroke(rect, 3.0, Stroke::new(1.5f32, YELLOW), StrokeKind::Inside);
        glow_rect(p, rect, 3.0, YELLOW, 4.0);
    } else if hovered {
        p.rect_filled(rect, 3.0, white_a(14));
        p.rect_stroke(rect, 3.0, Stroke::new(1.0f32, TEAL), StrokeKind::Inside);
    }
}

/// A name under a tile: centred, wrapped to two lines.
fn tile_name(p: &egui::Painter, rect: Rect, top: f32, text: &str, color: Color32) {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), bold(15.0), color, rect.width() - 10.0);
    job.halign = Align::Center;
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = false;
    let galley = p.layout_job(job);
    p.with_clip_rect(rect).galley(Pos2::new(rect.center().x, top), galley, color);
}

fn folder_tile(ui: &mut Ui, name: &str, count: usize, peek: &[Color32], selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(TILE, Sense::click());
    let p = ui.painter();
    tile_bg(p, rect, selected, resp.hovered());
    let icon = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 46.0), Vec2::new(92.0, 70.0));
    icons::folder_big(p, icon, peek);
    tile_name(p, rect, icon.bottom() + 8.0, name, if selected { YELLOW } else { WHITE });
    p.text(Pos2::new(rect.center().x, rect.bottom() - 6.0), Align2::CENTER_BOTTOM, if count == 1 { "1 game".to_string() } else { format!("{count} games") }, pixel(12.0), DIM);
    resp.on_hover_text(format!("{name} ({count})\nDouble-click to open"))
}

fn game_tile(ui: &mut Ui, t: &GameTile, cover: Option<egui::TextureId>, boxart: Option<(egui::TextureId, Vec2)>, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(TILE, Sense::click());
    let p = ui.painter();
    tile_bg(p, rect, selected, resp.hovered());
    let case = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 46.0), Vec2::new(56.0, 74.0));
    icons::game_case_art(p, case, platform_color(&t.platform), boxart, cover, t.status.color());
    tile_name(p, rect, case.bottom() + 6.0, &t.title, if selected { YELLOW } else { WHITE });
    let bar = Rect::from_min_max(Pos2::new(rect.left() + 14.0, rect.bottom() - 9.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 5.0));
    p.rect_filled(bar, 1.0, INK);
    p.rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.left() + bar.width() * t.progress, bar.bottom())), 1.0, t.status.color());
    resp.on_hover_text(format!("{}\n{} · {} · {:.0}%\nDouble-click to open", t.title, t.platform, t.status.label(), t.progress * 100.0))
}

const DETAIL_COLS: [(&str, f32); 5] = [("Name", 0.42), ("Platform", 0.18), ("Status", 0.13), ("Progress", 0.14), ("Last played", 0.13)];

fn details_header(ui: &mut Ui, w: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, PURPLE);
    let mut x = rect.left() + 6.0;
    for (name, frac) in DETAIL_COLS {
        p.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, name, bold(14.0), YELLOW);
        x += frac * w;
    }
}

#[allow(clippy::too_many_arguments)]
fn details_row(
    ui: &mut Ui,
    w: f32,
    n: usize,
    selected: bool,
    case_color: Option<Color32>,
    name: &str,
    platform: &str,
    status: &str,
    progress: Option<(f32, Color32)>,
    played: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
    let p = ui.painter();
    let bg = if selected {
        INDIGO
    } else if resp.hovered() {
        HOVER_BG
    } else if n % 2 == 1 {
        CARD_BG
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(rect, 0.0, bg);
    if selected {
        p.rect_stroke(rect.shrink(0.5), 0.0, Stroke::new(1.0f32, YELLOW), StrokeKind::Inside);
    }
    let cols: Vec<f32> = DETAIL_COLS.iter().scan(rect.left() + 6.0, |x, (_, f)| {
        let at = *x;
        *x += f * w;
        Some(at)
    }).collect();
    let cell = |i: usize| Rect::from_min_max(Pos2::new(cols[i], rect.top()), Pos2::new(cols.get(i + 1).copied().unwrap_or(rect.right()) - 8.0, rect.bottom()));
    // icon + name
    let icon = Rect::from_center_size(Pos2::new(cols[0] + 10.0, rect.center().y), Vec2::new(18.0, 16.0));
    match case_color {
        Some(c) => {
            let r = Rect::from_center_size(icon.center(), Vec2::new(12.0, 16.0));
            p.rect_filled(r, 2.0, Color32::from_rgb(0x2B, 0x27, 0x45));
            p.rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width(), 4.0)), 1.0, c);
            p.rect_stroke(r, 2.0, Stroke::new(1.0f32, Color32::from_gray(200)), StrokeKind::Inside);
        }
        None => icons::folder_icon(p, icon, false, true),
    }
    let name_cell = Rect::from_min_max(Pos2::new(cols[0] + 26.0, rect.top()), cell(0).max);
    p.with_clip_rect(name_cell).text(name_cell.left_center(), Align2::LEFT_CENTER, name, bold(15.0), if selected { YELLOW } else { WHITE });
    p.with_clip_rect(cell(1)).text(cell(1).left_center(), Align2::LEFT_CENTER, platform, pixel(14.0), TEAL);
    p.with_clip_rect(cell(2)).text(cell(2).left_center(), Align2::LEFT_CENTER, status, pixel(14.0), DIM);
    if let Some((f, c)) = progress {
        let c3 = cell(3);
        let bar = Rect::from_min_max(Pos2::new(c3.left(), c3.center().y - 3.0), Pos2::new(c3.right() - 40.0, c3.center().y + 3.0));
        p.rect_filled(bar, 1.0, INK);
        p.rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.left() + bar.width() * f, bar.bottom())), 1.0, c);
        p.text(Pos2::new(c3.right(), c3.center().y), Align2::RIGHT_CENTER, format!("{:.0}%", f * 100.0), pixel(13.0), TEAL);
    }
    p.with_clip_rect(cell(4)).text(cell(4).left_center(), Align2::LEFT_CENTER, played, pixel(13.0), DIM);
    resp
}

/// The right-click menu for a game (list, tree and explorer).
fn game_menu(ui: &mut Ui, id: u64, status: Status, folder: Option<Folder>, platform: &str, acts: &mut Vec<GameAct>) {
    if ui.button(RichText::new("Open").font(bold(15.0))).clicked() {
        acts.push(GameAct::Select(id));
        ui.close_menu();
    }
    ui.separator();
    ui.label(RichText::new("Set status").color(TEAL));
    for s in Status::ALL {
        if ui.selectable_label(status == s, s.label()).clicked() {
            acts.push(GameAct::Status(id, s));
            ui.close_menu();
        }
    }
    ui.separator();
    ui.label(RichText::new("Move to folder").color(TEAL));
    let auto = Folder::of_platform(platform);
    if ui.selectable_label(folder.is_none(), format!("Automatic ({})", auto.label())).clicked() {
        acts.push(GameAct::Folder(id, None));
        ui.close_menu();
    }
    for f in Folder::ALL {
        if ui.selectable_label(folder == Some(f), f.label()).clicked() {
            acts.push(GameAct::Folder(id, Some(f)));
            ui.close_menu();
        }
    }
}

/// The header's Trophies icon: the dashboard of every game's trophy progress.
fn trophies_icon(ui: &mut Ui, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(66.0, 56.0), Sense::click());
    let p = ui.painter();
    if active || resp.hovered() {
        p.rect_filled(rect, 3.0, if active { INDIGO } else { white_a(10) });
        p.rect_stroke(rect, 3.0, Stroke::new(1.0f32, if active { YELLOW } else { TEAL }), StrokeKind::Inside);
    }
    let ic = Rect::from_center_size(rect.center_top() + Vec2::new(0.0, 19.0), Vec2::splat(28.0));
    icons::trophy(p, ic, MAGENTA);
    let fg = if active { YELLOW } else if resp.hovered() { TEAL } else { WHITE };
    p.text(rect.center_bottom() - Vec2::new(0.0, 3.0), Align2::CENTER_BOTTOM, "Trophies", pixel(14.0), fg);
    resp.on_hover_text("Every game's trophy and achievement progress in one place")
}

/// The header's Missables icon: a warning sign.
fn missables_icon(ui: &mut Ui, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(66.0, 56.0), Sense::click());
    let p = ui.painter();
    if active || resp.hovered() {
        p.rect_filled(rect, 3.0, if active { INDIGO } else { white_a(10) });
        p.rect_stroke(rect, 3.0, Stroke::new(1.0f32, if active { YELLOW } else { TEAL }), StrokeKind::Inside);
    }
    let c = rect.center_top() + Vec2::new(0.0, 20.0);
    let tri = vec![c + Vec2::new(0.0, -13.0), c + Vec2::new(14.0, 11.0), c + Vec2::new(-14.0, 11.0)];
    p.add(egui::Shape::convex_polygon(tri, YELLOW, Stroke::new(2.0f32, MAGENTA)));
    p.rect_filled(Rect::from_center_size(c + Vec2::new(0.0, -1.0), Vec2::new(3.0, 11.0)), 0.0, INK);
    p.rect_filled(Rect::from_center_size(c + Vec2::new(0.0, 7.0), Vec2::splat(3.0)), 0.0, INK);
    let fg = if active { YELLOW } else if resp.hovered() { TEAL } else { WHITE };
    p.text(rect.center_bottom() - Vec2::new(0.0, 3.0), Align2::CENTER_BOTTOM, "Missables", pixel(14.0), fg);
    resp.on_hover_text("Everything you could still miss, across all your games")
}

/// The header's Explorer icon, next to the status icons.
fn explorer_icon(ui: &mut Ui, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(66.0, 56.0), Sense::click());
    let p = ui.painter();
    if active || resp.hovered() {
        p.rect_filled(rect, 3.0, if active { INDIGO } else { white_a(10) });
        p.rect_stroke(rect, 3.0, Stroke::new(1.0f32, if active { YELLOW } else { TEAL }), StrokeKind::Inside);
    }
    let ic = Rect::from_center_size(rect.center_top() + Vec2::new(0.0, 19.0), Vec2::new(34.0, 27.0));
    icons::folder_big(p, ic, &[Color32::from_rgb(0x2F, 0x5B, 0xD8), Color32::from_rgb(0xE6, 0x2A, 0x2A)]);
    let fg = if active { YELLOW } else if resp.hovered() { TEAL } else { WHITE };
    p.text(rect.center_bottom() - Vec2::new(0.0, 3.0), Align2::CENTER_BOTTOM, "Explorer", pixel(14.0), fg);
    resp.on_hover_text("Browse your games like a file explorer: folders, large icons or details")
}

/// What a row in the games list asks for.
enum GameAct {
    Select(u64),
    Status(u64, Status),
    Folder(u64, Option<Folder>),
}

/// The platform as a subfolder name: "ps5" and "PS5 " are the same folder.
fn platform_name(platform: &str) -> String {
    let p = platform.trim();
    if p.is_empty() {
        "No platform".into()
    } else {
        p.to_string()
    }
}

/// An explorer folder row: arrow, pixel folder, name and how many games are inside.
fn folder_row(ui: &mut Ui, name: &str, count: usize, open: bool, depth: usize, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 2.0, HOVER_BG);
    }
    let x = rect.left() + 6.0 + depth as f32 * 16.0;
    let k = Pos2::new(x + 4.0, rect.center().y);
    let tri = if open {
        vec![k + Vec2::new(-4.0, -2.5), k + Vec2::new(4.0, -2.5), k + Vec2::new(0.0, 3.0)]
    } else {
        vec![k + Vec2::new(-2.5, -4.0), k + Vec2::new(3.0, 0.0), k + Vec2::new(-2.5, 4.0)]
    };
    p.add(egui::Shape::convex_polygon(tri, if resp.hovered() { TEAL } else { YELLOW }, Stroke::NONE));
    let icon = Rect::from_center_size(Pos2::new(x + 22.0, rect.center().y), Vec2::new(20.0, 16.0));
    icons::folder_icon(p, icon, open, depth == 0);
    let col = if depth == 0 { YELLOW } else { TEAL };
    let text_rect = Rect::from_min_max(Pos2::new(icon.right() + 7.0, rect.top()), Pos2::new(rect.right() - 30.0, rect.bottom()));
    p.with_clip_rect(text_rect.intersect(p.clip_rect())).text(text_rect.left_center(), Align2::LEFT_CENTER, name, if depth == 0 { bold(16.0) } else { pixel(15.0) }, col);
    p.text(Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, count.to_string(), pixel(14.0), DIM);
    resp.on_hover_text(if open { "Close folder" } else { "Open folder" })
}

/// A game in the list: status stripe, title, details and progress. `indent` is in tree levels.
fn game_row(ui: &mut Ui, g: &Game, sel: bool, n: usize, indent: f32, width: f32, acts: &mut Vec<GameAct>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, GAME_ROW_H), Sense::click());
    let rect = Rect::from_min_max(Pos2::new(rect.left() + indent * 16.0, rect.top()), rect.max);
    let p = ui.painter();
    let bg = if sel {
        INDIGO
    } else if resp.hovered() {
        HOVER_BG
    } else if n % 2 == 1 {
        CARD_BG
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(rect, 0.0, bg);
    if sel {
        p.rect_stroke(rect.shrink(0.5), 0.0, Stroke::new(1.0f32, MAGENTA), StrokeKind::Inside);
        glow_rect(p, rect, 0.0, MAGENTA, 4.0);
    }
    p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height())), 0.0, g.status.color());

    let inner = rect.shrink2(Vec2::new(10.0, 4.0));
    let clip = p.with_clip_rect(inner.intersect(p.clip_rect()));
    clip.text(inner.left_top() + Vec2::new(2.0, 0.0), Align2::LEFT_TOP, &g.title, bold(16.0), if sel { YELLOW } else { WHITE });
    let mut sub = vec![g.status.label().to_string()];
    if !g.platform.is_empty() {
        sub.push(g.platform.clone());
    }
    sub.push(match g.guides.len() {
        1 => "1 guide".into(),
        n => format!("{n} guides"),
    });
    if let Some(t) = g.last_played {
        sub.push(model::ago(t));
    }
    clip.text(inner.left_top() + Vec2::new(2.0, 19.0), Align2::LEFT_TOP, sub.join(" · "), pixel(12.0), DIM);

    let pr = g.progress();
    let bar = Rect::from_min_max(Pos2::new(inner.left() + 2.0, inner.bottom() - 4.0), Pos2::new(inner.right() - 34.0, inner.bottom() - 1.0));
    p.rect_filled(bar, 1.0, INK);
    p.rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.left() + bar.width() * pr, bar.bottom())), 1.0, g.status.color());
    p.text(Pos2::new(inner.right(), bar.center().y), Align2::RIGHT_CENTER, format!("{:.0}%", pr * 100.0), pixel(12.0), TEAL);

    if resp.clicked() {
        acts.push(GameAct::Select(g.id));
    }
    resp.context_menu(|ui| game_menu(ui, g.id, g.status, g.folder, &g.platform, acts));
}

/// Everything that decides a checklist row's height. If it's unchanged, the remembered height holds.
fn row_fingerprint(s: &Step, look: &RowLook, width: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&s.text, &s.detail, &s.how, &s.reward, &s.deadline, s.optional, s.hidden).hash(&mut h);
    (&s.note, &s.image, s.rarity.map(f32::to_bits)).hash(&mut h);
    (look.state as u8, look.is_next, look.can_find, width.round() as i32, look.compact, look.open, look.veiled, look.thumb.map(|t| (t.1.x as i32, t.1.y as i32))).hash(&mut h);
    h.finish()
}

/// An item's image, loaded (scaled down) the first time it's shown.
fn thumb_of(ctx: &egui::Context, thumbs: &mut HashMap<String, Fav>, path: &str) -> Option<(egui::TextureId, Vec2)> {
    let entry = thumbs.entry(path.to_string()).or_insert_with(|| match image::open(path) {
        Ok(img) => {
            let img = img.thumbnail(640, 360).to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            Fav::Ready(ctx.load_texture(format!("thumb:{path}"), ci, egui::TextureOptions::LINEAR))
        }
        Err(_) => Fav::Missing,
    });
    match entry {
        Fav::Ready(t) => Some((t.id(), t.size_vec2())),
        _ => None,
    }
}

/// Rarity colours: under 5% is rare, under 20% uncommon.
fn rarity_color(pct: f32) -> Color32 {
    if pct < 5.0 {
        MAGENTA
    } else if pct < 20.0 {
        YELLOW
    } else {
        DIM
    }
}

/// One "Search everything" result.
#[derive(Clone, Debug)]
struct SearchHit {
    game: u64,
    guide: Option<u64>,
    item: Option<usize>,
    title: String,
    /// "Game > Guide > Area"
    place: String,
    snippet: String,
    done: bool,
}

/// Games, guides and checklist items matching `q` (lower case), up to `limit`.
fn search_library(lib: &Library, q: &str, limit: usize) -> Vec<SearchHit> {
    let has = |t: &str| t.to_lowercase().contains(q);
    let snip = |t: &str| {
        let lower = t.to_lowercase();
        let at = lower.find(q).unwrap_or(0);
        let start = t.char_indices().map(|(i, _)| i).filter(|i| *i <= at.saturating_sub(40)).last().unwrap_or(0);
        let text: String = t[start..].chars().take(140).collect();
        if start > 0 { format!("\u{2026}{text}") } else { text }
    };
    let mut out = Vec::new();
    for g in &lib.games {
        if has(&g.title) || has(&g.platform) || has(&g.resume_note) || g.tags.iter().any(|t| has(t)) {
            out.push(SearchHit { game: g.id, guide: None, item: None, title: g.title.clone(), place: format!("Game \u{b7} {}", g.platform), snippet: String::new(), done: false });
        }
        for gd in &g.guides {
            if has(&gd.title) {
                out.push(SearchHit { game: g.id, guide: Some(gd.id), item: None, title: gd.title.clone(), place: format!("{} \u{b7} guide", g.title), snippet: String::new(), done: gd.done });
            }
            for (i, s) in gd.steps.iter().enumerate() {
                if out.len() >= limit {
                    return out;
                }
                let field = [&s.text, &s.detail, &s.how, &s.reward, &s.section, &s.note].into_iter().find(|t| has(t));
                if let Some(f) = field {
                    let place = if s.section.is_empty() { format!("{} > {}", g.title, gd.title) } else { format!("{} > {} > {}", g.title, gd.title, s.section) };
                    let snippet = if f == &s.text { String::new() } else { snip(f) };
                    out.push(SearchHit { game: g.id, guide: Some(gd.id), item: Some(i), title: s.text.clone(), place, snippet, done: s.done });
                }
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TickMode {
    Named,
    AllExcept,
    Untick,
}

/// For matching names: lower case, letters and digits only, single spaces.
fn name_key(s: &str) -> String {
    let lowered: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Items whose name appears as a whole line of `text` (ignoring case and punctuation). Item names
/// like "#12 Pagan's Dilemma" or "Achievement: Shake on It" also match on the part after the prefix.
fn match_names(steps: &[Step], text: &str) -> Vec<usize> {
    let lines: HashSet<String> = text.lines().map(name_key).filter(|k| !k.is_empty()).collect();
    steps
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            let full = name_key(&s.text);
            let after_prefix = s.text.split_once(": ").map(|(_, rest)| name_key(rest));
            let after_number = s.text.strip_prefix('#').and_then(|r| r.split_once(' ')).map(|(_, rest)| name_key(rest));
            lines.contains(&full) || after_prefix.is_some_and(|k| lines.contains(&k)) || after_number.is_some_and(|k| lines.contains(&k))
        })
        .map(|(i, _)| i)
        .collect()
}

impl Reader {
    fn new(guide: u64, text: &str) -> Self {
        // runs of blank lines become one
        let mut lines: Vec<String> = Vec::new();
        for l in text.lines() {
            let l = l.trim_end();
            if l.trim().is_empty() && lines.last().is_none_or(|p| p.trim().is_empty()) {
                continue;
            }
            lines.push(l.to_string());
        }
        Reader { guide, heights: Vec::new(), width: 0.0, lines, target: None, target_key: "-".into(), jump: None, found: Vec::new(), found_at: 0, found_for: String::new() }
    }
}

/// A line's height before it has been laid out (about 8 px per character at 16 px).
fn estimate_line(line: &str, width: f32) -> f32 {
    if line.trim().is_empty() {
        return 10.0;
    }
    let per_row = (width / 8.0).max(10.0);
    let rows = (line.chars().count() as f32 / per_row).ceil().max(1.0);
    rows * if line.starts_with("## ") { 22.0 } else { 19.0 } + 6.0
}

/// For comparing a page with a checklist: lower case, straight quotes and dashes, single spaces.
fn norm_text(s: &str) -> String {
    let s: String = s
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{2032}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{2033}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            c => c,
        })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The line of a guide's text an item is on: its anchor or name, searched for after its area's
/// heading (so "Chest 1" finds the one in the right area).
fn find_line(lines: &[String], section: &str, text: &str, anchor: &str) -> Option<usize> {
    let lower: Vec<String> = lines.iter().map(|l| norm_text(l)).collect();
    let sec = norm_text(section);
    let start = if sec.is_empty() { 0 } else { lower.iter().position(|l| l.contains(&sec)).unwrap_or(0) };
    let mut needles: Vec<String> = Vec::new();
    // long items (route steps) are often reflowed on the page: their opening words still match
    let opening: String = text.chars().take(36).collect();
    for n in [anchor, text, text.split_once(": ").map_or("", |x| x.1), text.strip_prefix('#').and_then(|r| r.split_once(' ')).map_or("", |x| x.1), opening.as_str()] {
        let n = norm_text(n);
        if n.chars().count() >= 3 && !needles.contains(&n) {
            needles.push(n);
        }
    }
    for n in &needles {
        if let Some(p) = lower[start..].iter().position(|l| l.contains(n.as_str())) {
            return Some(start + p);
        }
    }
    needles.iter().find_map(|n| lower.iter().position(|l| l.contains(n.as_str())))
}

/// "2 chests · 1 objective" for the given items.
fn kind_counts(steps: &[Step], idx: impl Iterator<Item = usize>) -> String {
    let mut n = [0usize; 4];
    for i in idx {
        n[StepKind::ALL.iter().position(|k| *k == steps[i].kind).unwrap_or(3)] += 1;
    }
    let names = [("chest", "chests"), ("objective", "objectives"), ("achievement", "achievements"), ("item", "items")];
    StepKind::ALL
        .iter()
        .enumerate()
        .filter(|(i, _)| n[*i] > 0)
        .map(|(i, _)| format!("{} {}", n[i], if n[i] == 1 { names[i].0 } else { names[i].1 }))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A small outlined tag, e.g. "optional" or "Ends 9/25".
fn tag(ui: &mut Ui, text: &str, col: Color32) {
    egui::Frame::new().fill(INK).stroke(Stroke::new(1.0f32, col)).corner_radius(2).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).small().color(col));
    });
}

/// One checklist item: tick box, type icon, name, requirement, How and Reward.
/// Clicking anywhere on the row ticks it (or, for a spoiler-guarded item, shows it).
fn step_row(ui: &mut Ui, s: &Step, i: usize, look: &RowLook, acts: &mut Vec<(usize, RowAct)>) -> egui::Response {
    let RowLook { state, is_next, can_find, compact, open, cursor, veiled, thumb } = *look;
    let has_details = !veiled && (!s.detail.is_empty() || !s.how.is_empty() || !s.note.is_empty() || !s.image.is_empty());
    // compact rows show their details only when opened
    let show_details = !compact || open;
    let done = state == StepState::Done;
    let missed = state == StepState::Missed;
    let faded = done || missed;
    let resp = ui
        .scope_builder(egui::UiBuilder::new().id_salt(("step-row", i)).sense(Sense::click()), |ui| {
            let hovered = ui.response().hovered();
            let fill = if hovered {
                HOVER_BG
            } else if faded {
                Color32::from_rgb(0x1c, 0x1b, 0x2c)
            } else {
                CARD_BG
            };
            let stroke = if cursor {
                // the keyboard highlight
                Stroke::new(2.5f32, TEAL)
            } else if is_next {
                Stroke::new(2.0f32, MAGENTA)
            } else if state == StepState::DueSoon {
                Stroke::new(1.5f32, MAGENTA.gamma_multiply(0.6))
            } else {
                Stroke::new(1.0f32, if faded { Color32::from_rgb(0x2a, 0x26, 0x48) } else { INDIGO })
            };
            let margin = if compact { egui::Margin::symmetric(8, 3) } else { egui::Margin::symmetric(10, 8) };
            egui::Frame::new().fill(fill).stroke(stroke).corner_radius(3).inner_margin(margin).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    if big_checkbox(ui, done).on_hover_text(if done { "Untick" } else { "Tick off" }).clicked() {
                        acts.push((i, RowAct::Toggle));
                    }
                    ui.add_space(4.0);
                    let (ir, _) = ui.allocate_exact_size(Vec2::new(22.0, 24.0), Sense::hover());
                    icons::step_icon(ui.painter(), Rect::from_center_size(ir.center(), Vec2::splat(20.0)), s.kind, faded);
                    if compact {
                        // the arrow that opens this row's details
                        let (ar, aresp) = ui.allocate_exact_size(Vec2::new(16.0, 24.0), if has_details { Sense::click() } else { Sense::hover() });
                        if has_details {
                            let col = if aresp.hovered() { TEAL } else { YELLOW };
                            let k = ar.center();
                            let tri = if open {
                                vec![k + Vec2::new(-4.0, -2.0), k + Vec2::new(4.0, -2.0), k + Vec2::new(0.0, 3.0)]
                            } else {
                                vec![k + Vec2::new(-2.0, -4.0), k + Vec2::new(3.0, 0.0), k + Vec2::new(-2.0, 4.0)]
                            };
                            ui.painter().add(egui::Shape::convex_polygon(tri, col, Stroke::NONE));
                            if aresp.on_hover_text(if open { "Hide details" } else { "Show details" }).clicked() {
                                acts.push((i, RowAct::Details));
                            }
                        }
                    } else {
                        ui.add_space(4.0);
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if can_find && !veiled && ui.small_button("Find in guide").on_hover_text("Open the guide scrolled to this item").clicked() {
                                    acts.push((i, RowAct::Find));
                                }
                                if !s.reward.is_empty() && !veiled {
                                    tag(ui, &format!("Reward: {}", s.reward), if faded { DIM } else { TEAL });
                                }
                                if let Some(r) = s.rarity {
                                    let col = if faded { DIM } else { rarity_color(r) };
                                    tag(ui, &format!("{r:.1}%"), col);
                                }
                                if !s.note.is_empty() && compact && !open && !veiled {
                                    tag(ui, "note", TEAL);
                                }
                                match state {
                                    StepState::Missed => tag(ui, "Missed", Color32::from_rgb(0xE0, 0x60, 0x60)),
                                    StepState::DueSoon => tag(ui, &format!("Ends {}", s.deadline), MAGENTA),
                                    StepState::Open if !s.deadline.is_empty() => tag(ui, &format!("Ends {}", s.deadline), DIM),
                                    _ => {}
                                }
                                if s.optional {
                                    tag(ui, "optional", DIM);
                                }
                                if is_next {
                                    ui.label(RichText::new("NEXT").font(bold(14.0)).color(MAGENTA));
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    let col = if faded {
                                        DIM
                                    } else if is_next {
                                        MAGENTA
                                    } else {
                                        WHITE
                                    };
                                    let mut t = RichText::new(&s.text).font(bold(if compact { 16.0 } else { 18.0 })).color(col);
                                    if done {
                                        t = t.strikethrough();
                                    }
                                    if veiled {
                                        t = RichText::new("\u{2592}\u{2592}\u{2592}\u{2592}\u{2592}\u{2592}  hidden by the spoiler guard: click to show").font(pixel(if compact { 15.0 } else { 16.0 })).color(DIM);
                                    }
                                    let label = egui::Label::new(t).selectable(false);
                                    ui.add(if compact && !open { label.truncate() } else { label.wrap() });
                                    if s.hidden {
                                        ui.label(RichText::new("hidden").small().color(DIM));
                                    }
                                });
                            });
                        });
                        // Finished items keep their full height (just dimmed) so nothing below
                        // them slides up under the mouse. "Hide done" removes them instead.
                        let show_details = show_details && !veiled;
                        if show_details && !s.detail.is_empty() {
                            ui.add(egui::Label::new(RichText::new(&s.detail).color(if faded { DIM } else { YELLOW })).wrap().selectable(false));
                        }
                        if show_details && !s.how.is_empty() {
                            ui.add_space(2.0);
                            let col = if faded { DIM.gamma_multiply(0.7) } else { DIM };
                            ui.add(egui::Label::new(RichText::new(format!("How: {}", s.how)).color(col)).wrap().selectable(false));
                        }
                        if show_details && !s.note.is_empty() {
                            ui.add_space(2.0);
                            ui.add(egui::Label::new(RichText::new(format!("Note: {}", s.note)).color(if faded { DIM } else { TEAL })).wrap().selectable(false));
                        }
                        if show_details && !s.image.is_empty() {
                            ui.add_space(2.0);
                            match thumb {
                                Some((tex, size)) => {
                                    let h = 120.0f32;
                                    let w = (size.x / size.y.max(1.0) * h).min(ui.available_width());
                                    let r = ui
                                        .add(egui::Image::new((tex, Vec2::new(w, w / size.x.max(1.0) * size.y))).sense(Sense::click()))
                                        .on_hover_text("Open the full image");
                                    if r.clicked() {
                                        acts.push((i, RowAct::OpenImage));
                                    }
                                }
                                None => {
                                    ui.label(RichText::new(format!("Image not found: {}", s.image)).small().color(MAGENTA));
                                }
                            }
                        }
                    });
                });
            });
        })
        .response;
    let resp = if compact && !open && has_details {
        let mut tip = s.detail.clone();
        for (label, text) in [("How", &s.how), ("Note", &s.note)] {
            if !text.is_empty() {
                if !tip.is_empty() {
                    tip.push_str("\n\n");
                }
                tip.push_str(&format!("{label}: {text}"));
            }
        }
        resp.on_hover_ui(|ui| {
            ui.set_max_width(460.0);
            ui.label(tip);
        })
    } else {
        resp
    };
    if is_next {
        glow_rect(ui.painter(), resp.rect, 3.0, MAGENTA, 5.0);
    }
    if resp.clicked() {
        acts.push((i, if veiled { RowAct::Reveal } else { RowAct::Toggle }));
    }
    resp.context_menu(|ui| {
        if ui.button(if done { "Untick" } else { "Tick off" }).clicked() {
            acts.push((i, RowAct::Toggle));
            ui.close_menu();
        }
        if veiled && ui.button("Show it (spoiler guard)").clicked() {
            acts.push((i, RowAct::Reveal));
            ui.close_menu();
        }
        if ui.button(if s.note.is_empty() { "Add a note\u{2026}" } else { "Edit the note\u{2026}" }).clicked() {
            acts.push((i, RowAct::Note));
            ui.close_menu();
        }
        if ui.button(if s.image.is_empty() { "Attach an image\u{2026}" } else { "Change the image\u{2026}" }).on_hover_text("A screenshot or map for this item").clicked() {
            acts.push((i, RowAct::Image));
            ui.close_menu();
        }
        if !s.image.is_empty() && ui.button("Remove the image").clicked() {
            acts.push((i, RowAct::NoImage));
            ui.close_menu();
        }
        ui.separator();
        let missed_label = if s.missed { "Not missed after all" } else { "Mark as missed / skipped" };
        if !s.missed && missed && !s.deadline.is_empty() {
            ui.label(RichText::new(format!("Past its deadline ({})", s.deadline)).small().color(DIM));
        } else if ui.button(missed_label).clicked() {
            acts.push((i, RowAct::Missed));
            ui.close_menu();
        }
        if ui.button("Tick everything before this").on_hover_text("For \"I'm up to here\": ticks every item above this one").clicked() {
            acts.push((i, RowAct::TickBefore));
            ui.close_menu();
        }
        if ui.button("Untick this and everything after").clicked() {
            acts.push((i, RowAct::UntickFrom));
            ui.close_menu();
        }
        if ui.button(if s.optional { "Mark as story" } else { "Mark as optional" }).clicked() {
            acts.push((i, RowAct::Optional));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Move up").clicked() {
            acts.push((i, RowAct::Up));
            ui.close_menu();
        }
        if ui.button("Move down").clicked() {
            acts.push((i, RowAct::Down));
            ui.close_menu();
        }
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(s.text.clone());
            ui.close_menu();
        }
        ui.separator();
        if ui.button(RichText::new("Delete item").color(MAGENTA)).clicked() {
            acts.push((i, RowAct::Delete));
            ui.close_menu();
        }
    });
    resp
}

/// "Add an item" box at the bottom of a checklist. New items go in the last area.
fn add_item_row(ui: &mut Ui, g: &mut Guide, buf: &mut String) {
    let (r, add, enter) = input_row(ui, buf, "Add an item: chapter, collectible, trophy…", ("new-step", g.id), "+ Add");
    if (add || enter) && !buf.trim().is_empty() {
        let section = g.steps.last().map(|s| s.section.clone()).unwrap_or_default();
        g.steps.push(Step { section, ..Step::task(checklist::plain_item(buf)) });
        buf.clear();
        if enter {
            r.request_focus();
        }
    }
}

/// Two-step button so a stray click can't delete things: the first click arms it for 3 s.
fn armed_button(ui: &mut Ui, key: impl Hash, label: &str, now: f64) -> bool {
    let id = ui.id().with(key);
    let armed = ui.ctx().data(|d| d.get_temp::<f64>(id)).is_some_and(|t| now < t);
    let text = if armed { RichText::new("Sure?").color(MAGENTA) } else { RichText::new(label) };
    let mut fire = false;
    if ui.button(text).clicked() {
        if armed {
            fire = true;
            ui.ctx().data_mut(|d| d.remove::<f64>(id));
        } else {
            ui.ctx().data_mut(|d| d.insert_temp(id, now + 3.0));
        }
    }
    fire
}

/// A text box that fills the row, with a button at its right end. The button is laid out first
/// so the box takes exactly the space left: the row can never be wider than where it sits.
/// Returns (text box, button clicked, Enter pressed in the box).
fn input_row(ui: &mut Ui, buf: &mut String, hint: &str, id: impl Hash, button: &str) -> (egui::Response, bool, bool) {
    // exactly one row tall: a bare `with_layout` would take (and centre itself in) all the
    // height left below it, dropping the row into the middle of the panel
    let row = Vec2::new(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
        let clicked = ui.button(button).clicked();
        let r = ui.add(egui::TextEdit::singleline(buf).id_salt(id).hint_text(hint).desired_width(ui.available_width()));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        (r, clicked, enter)
    })
    .inner
}

/// Draws `add_contents` in a box exactly the size of the space left in `ui`, and reports
/// exactly that size back, however wide the contents turn out.
///
/// A resizable side panel remembers its width as the width of whatever was drawn in it. Without
/// this, any row that's a few pixels too wide (a button row, a long title) makes the panel grow
/// on its own and snap back when you drag it narrower. Overflow is clipped instead.
fn contained(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    let rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add_contents(&mut child);
    #[cfg(test)]
    tests::OVERFLOW.with(|o| o.set(o.get().max(child.min_rect().width() - rect.width())));
    ui.allocate_rect(rect, Sense::hover());
}

fn vline(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 28.0), Sense::hover());
    ui.painter().rect_filled(Rect::from_center_size(r.center(), Vec2::new(2.0, 26.0)), 0.0, YELLOW);
    ui.painter().rect_filled(Rect::from_center_size(r.center() + Vec2::new(2.0, 0.0), Vec2::new(1.0, 26.0)), 0.0, WHITE);
}

/// A pixel icon with a label under it, like the site's desktop shortcuts.
fn desktop_icon(ui: &mut Ui, which: Option<Status>, active: bool, count: usize) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(62.0, 56.0), Sense::click());
    let p = ui.painter();
    if active || resp.hovered() {
        p.rect_filled(rect, 3.0, if active { INDIGO } else { white_a(10) });
        p.rect_stroke(rect, 3.0, Stroke::new(1.0f32, if active { YELLOW } else { TEAL }), StrokeKind::Inside);
    }
    let ic = Rect::from_center_size(rect.center_top() + Vec2::new(0.0, 19.0), Vec2::splat(30.0));
    icons::filter_icon(p, ic, which);
    if count > 0 {
        let at = ic.right_top() + Vec2::new(4.0, 2.0);
        let txt = count.to_string();
        let w = 8.0 + 6.0 * txt.len() as f32;
        let badge = Rect::from_center_size(at, Vec2::new(w, 13.0));
        p.rect_filled(badge, 6.0, MAGENTA);
        p.text(badge.center(), Align2::CENTER_CENTER, txt, pixel(12.0), INK);
    }
    let label = match which {
        None => "All",
        Some(Status::Completed) => "Done",
        Some(s) => s.label(),
    };
    let fg = if active { YELLOW } else if resp.hovered() { TEAL } else { WHITE };
    p.text(rect.center_bottom() - Vec2::new(0.0, 3.0), Align2::CENTER_BOTTOM, label, pixel(14.0), fg);
    resp.on_hover_text(match which {
        None => format!("All games ({count})"),
        Some(s) => format!("{} ({count})", s.label()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::containers::panel::PanelState;

    thread_local! {
        /// How far side-panel contents ran past the panel's edge (see `contained`).
        pub static OVERFLOW: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
    }

    /// A library with long titles, notes and button rows: the kind of content that used to
    /// push the side panels wider than you set them.
    fn sample() -> Library {
        let mut lib = Library::default();
        let steps = (0..40)
            .map(|i| Step {
                text: format!("Objective {i}: A rather long objective name that keeps going and going"),
                section: format!("Area {} (cleared data, part 2)", i / 10),
                detail: "Do the thing in the place with the other thing, then come back.".into(),
                how: "A long explanation of how to do it. ".repeat(6),
                reward: "Lustrous Arm Guards".into(),
                deadline: if i % 3 == 0 { "9/25".into() } else { String::new() },
                optional: i % 4 == 0,
                done: i < 12,
                ..Default::default()
            })
            .collect();
        let guide = Guide {
            id: 2,
            title: "An extremely long guide title | With A Site Name | And Even More Words In It".into(),
            url: "https://example.com/a/very/long/guide/address/that/goes/on".into(),
            steps,
            ..Default::default()
        };
        let link = Guide { id: 3, title: "Reference link with a long name too".into(), url: "https://example.org/x".into(), ..Default::default() };
        lib.games.push(Game {
            id: 1,
            title: "KINGDOM HEARTS HD 2.8 Final Chapter Prologue With Extra Words".into(),
            platform: "PC (Steam)".into(),
            resume_note: "Where I left off: ".repeat(20),
            log: (0..5).map(|i| LogEntry { at: 1_700_000_000 + i * 86_400, text: "Session notes that are fairly long. ".repeat(4) }).collect(),
            guides: vec![guide, link],
            game_date: "4/2".into(),
            tags: vec!["100%".into(), "co-op".into()],
            ..Default::default()
        });
        lib.games.push(Game { id: 4, title: "Fire Emblem: Fortune's Weave".into(), platform: "Switch 2".into(), ..Default::default() });
        lib.next_id = 10;
        lib
    }

    fn app(ctx: &egui::Context) -> App {
        install(ctx);
        let saved = Saved { online: false, selected: Some(1), ..Default::default() };
        let nowhere = std::env::temp_dir().join("jagt-test-does-not-exist").join("library.json");
        App::build(ctx, saved, sample(), nowhere, false, false)
    }

    fn run(app: &mut App, ctx: &egui::Context, size: Vec2, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let _ = ctx.run(input, |ctx| app.frame(ctx));
    }

    fn width(ctx: &egui::Context, id: &str) -> f32 {
        PanelState::load(ctx, egui::Id::new(id)).expect("panel shown").rect.width()
    }

    fn drag(app: &mut App, ctx: &egui::Context, size: Vec2, from: Pos2, to: Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(app, ctx, size, vec![egui::Event::PointerMoved(from)]);
        run(app, ctx, size, vec![button(from, true)]);
        for k in 1..=6 {
            run(app, ctx, size, vec![egui::Event::PointerMoved(from.lerp(to, k as f32 / 6.0))]);
        }
        run(app, ctx, size, vec![button(to, false)]);
        run(app, ctx, size, vec![egui::Event::PointerGone]);
    }

    /// Frame-time benchmark on a real library. Not run by default:
    /// `JAGT_BENCH_LIB=path/to/copy.json cargo test --release bench_frames -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_frames() {
        let Ok(path) = std::env::var("JAGT_BENCH_LIB") else {
            eprintln!("set JAGT_BENCH_LIB to a copy of library.json");
            return;
        };
        let lib = model::read_file(std::path::Path::new(&path)).expect("library");
        let biggest = lib.games.iter().max_by_key(|g| g.guides.iter().map(|x| x.steps.len()).sum::<usize>()).map(|g| g.id);
        let ctx = egui::Context::default();
        install(&ctx);
        let saved = Saved { online: false, selected: biggest, ..Default::default() };
        let nowhere = std::env::temp_dir().join("jagt-bench-does-not-exist").join("library.json");
        let mut app = App::build(&ctx, saved, lib, nowhere, false, false);
        let size = Vec2::new(1700.0, 1000.0);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
            let out = ctx.run(input, |ctx| app.frame(ctx));
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
        };
        for _ in 0..20 {
            frame(&mut app, vec![]);
        }
        let time = |label: &str, app: &mut App, make: &dyn Fn(usize) -> Vec<egui::Event>| {
            let n = 200;
            let t = Instant::now();
            for k in 0..n {
                frame(app, make(k));
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            eprintln!("BENCH {label:<28} {ms:7.2} ms/frame  ({:.0} fps)", 1000.0 / ms);
        };
        // where the time goes
        {
            let n = 100;
            let (mut run_t, mut tess_t) = (0.0, 0.0);
            for _ in 0..n {
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
                let t = Instant::now();
                let out = ctx.run(input, |ctx| app.frame(ctx));
                run_t += t.elapsed().as_secs_f64();
                let t = Instant::now();
                let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
                tess_t += t.elapsed().as_secs_f64();
            }
            eprintln!("BENCH   layout {:.2} ms, tessellate {:.2} ms", run_t * 1000.0 / n as f64, tess_t * 1000.0 / n as f64);
            let t = Instant::now();
            for _ in 0..n {
                std::hint::black_box(hash_lib(&app.lib));
            }
            eprintln!("BENCH   unsaved-changes check (hash_lib) {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
        time("idle", &mut app, &|_| vec![]);
        time("mouse moving over list", &mut app, &|k| vec![egui::Event::PointerMoved(Pos2::new(700.0 + (k % 50) as f32 * 4.0, 750.0))]);
        time("scrolling the checklist", &mut app, &|_| {
            vec![
                egui::Event::PointerMoved(Pos2::new(800.0, 800.0)),
                egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -40.0), modifiers: Default::default() },
            ]
        });
    }

    #[test]
    fn dropping_a_folder_adds_its_guide_files() {
        let dir = std::env::temp_dir().join(format!("jagt-drop-test-{}", std::process::id()));
        let sub = dir.join("maps");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("Walkthrough Part 1.pdf"), b"%PDF").unwrap();
        std::fs::write(sub.join("World Map.png"), b"png").unwrap();
        std::fs::write(dir.join("game.exe"), b"MZ").unwrap();

        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let before = app.lib.games[0].guides.len();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 850.0))),
            dropped_files: vec![egui::DroppedFile { path: Some(dir.clone()), ..Default::default() }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.frame(ctx));

        let guides = &app.lib.games[0].guides;
        let added: Vec<(&str, Kind)> = guides[before..].iter().map(|g| (g.title.as_str(), g.kind)).collect();
        assert_eq!(added, [("World Map", Kind::Map), ("Walkthrough Part 1", Kind::Doc)], "the .exe is skipped, subfolders are searched");
        assert!(guides[before..].iter().all(|g| links::is_local(&g.url)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explorer_folders_and_paths() {
        let g = |id, platform: &str| Game { id, title: format!("game {id}"), platform: platform.into(), ..Default::default() };
        let all = [g(1, "PS5"), g(2, "PSP"), g(3, "ps5"), g(4, "PC (Steam)"), g(5, "Android")];
        let games: Vec<&Game> = all.iter().collect();
        let names = |v: &Vec<(String, Vec<u64>)>| v.iter().map(|(n, ids)| (n.clone(), ids.len())).collect::<Vec<_>>();

        let (top, loose) = explorer_children(&games, &[]).unwrap();
        assert_eq!(names(&top), [("PC".to_string(), 1), ("Console".into(), 3), ("Mobile".into(), 1)]);
        assert!(loose.is_empty());
        // Console holds two platforms ("PS5" and "ps5" are the same folder) → subfolders
        let (sub, loose) = explorer_children(&games, &["Console".into()]).unwrap();
        assert_eq!(names(&sub), [("PS5".to_string(), 2), ("PSP".into(), 1)]);
        assert!(loose.is_empty());
        assert_eq!(explorer_children(&games, &["Console".into(), "PS5".into()]).unwrap().1, vec![1, 3]);
        // a single-platform folder lists its games directly
        assert_eq!(explorer_children(&games, &["PC".into()]).unwrap(), (vec![], vec![4]));
        // paths that don't exist
        assert!(explorer_children(&games, &["Linux".into()]).is_none() || explorer_children(&games, &["Linux".into()]).unwrap().1.is_empty());
        assert!(explorer_children(&games, &["Console".into(), "Xbox".into()]).is_none());
        assert!(explorer_children(&games, &["Nope".into()]).is_none());
    }

    #[test]
    fn explorer_opens_and_navigates() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 850.0);
        app.explorer_open = true;
        for layout in [ExplorerLayout::Icons, ExplorerLayout::Details] {
            app.ex_layout = layout;
            for path in [vec![], vec!["Console".to_string()], vec!["PC".to_string()]] {
                app.ex_go(path);
                for _ in 0..3 {
                    run(&mut app, &ctx, size, vec![]);
                }
            }
        }
        // back goes to the previous folder, a missing folder falls back to the top
        app.ex_path = vec!["Nope".into()];
        run(&mut app, &ctx, size, vec![]);
        assert!(app.ex_path.is_empty());
        // opening a game closes the explorer and selects it
        app.ex_open_game(4);
        assert_eq!((app.explorer_open, app.selected), (false, Some(4)));
    }

    #[test]
    fn focus_and_compact_rows() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 850.0);
        let checklist_height = |app: &mut App, ctx: &egui::Context| {
            for _ in 0..3 {
                run(app, ctx, size, vec![]);
            }
            // the first item's remembered height
            app.row_heights.get(&(2, 0)).map(|h| h.1).unwrap_or(0.0)
        };
        let tall = checklist_height(&mut app, &ctx);
        app.compact_rows = true;
        let short = checklist_height(&mut app, &ctx);
        assert!(short > 0.0 && short < tall * 0.6, "compact row {short}px vs normal {tall}px");
        // opening a compact row's details makes it taller again
        app.open_rows.insert((2, 0));
        let opened = checklist_height(&mut app, &ctx);
        assert!(opened > short + 20.0, "opened row {opened}px vs closed {short}px");
        // every single-window view draws (Focus = the Checklist view), and Esc goes back to all
        for v in ViewMode::ALL {
            app.view = v;
            for _ in 0..2 {
                run(&mut app, &ctx, size, vec![]);
            }
        }
        app.view = ViewMode::Checklist;
        run(&mut app, &ctx, size, vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert_eq!(app.view, ViewMode::All);
    }

    /// The □ in a window's title bar: where it is in the drawn frame (found by its 12×12 box on the
    /// same line as the window's title).
    fn maximize_button(ctx: &egui::Context, app: &mut App, size: Vec2, title: &str) -> Pos2 {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
        let out = ctx.run(input, |ctx| app.frame(ctx));
        // the same word can appear elsewhere (the header's Show buttons): the window title is the
        // one with the three 12×12 title-bar boxes on its line
        let boxes_on = |y: f32| {
            let mut boxes: Vec<Rect> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Rect(r) if (r.rect.width() - 12.0).abs() < 0.5 && (r.rect.height() - 12.0).abs() < 0.5 && (r.rect.center().y - y).abs() < 4.0 => Some(r.rect),
                    _ => None,
                })
                .collect();
            boxes.sort_by(|a, b| b.center().x.total_cmp(&a.center().x));
            boxes.dedup_by(|a, b| (a.center().x - b.center().x).abs() < 1.0);
            boxes
        };
        let boxes = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == title => Some(boxes_on(t.pos.y + t.galley.size().y / 2.0)),
                _ => None,
            })
            .find(|b| b.len() >= 3)
            .expect("window title with its title-bar buttons");
        // right to left: ×, □, _
        boxes[1].center()
    }

    #[test]
    fn title_bar_maximize_and_restore() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 850.0);
        for _ in 0..3 {
            run(&mut app, &ctx, size, vec![]);
        }
        let click = |app: &mut App, at: Pos2| {
            let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            run(app, &ctx, size, vec![egui::Event::PointerMoved(at)]);
            run(app, &ctx, size, vec![b(true)]);
            run(app, &ctx, size, vec![b(false)]);
            run(app, &ctx, size, vec![egui::Event::PointerGone]);
        };
        let at = maximize_button(&ctx, &mut app, size, "Checklist");
        click(&mut app, at);
        assert_eq!(app.view, ViewMode::Checklist, "□ on the Checklist shows only the Checklist");
        let at = maximize_button(&ctx, &mut app, size, "Checklist");
        click(&mut app, at);
        assert_eq!(app.view, ViewMode::All, "the restore button goes back to all windows");
        // the journal's windows maximise to the Notes view
        let at = maximize_button(&ctx, &mut app, size, "Session Log");
        click(&mut app, at);
        assert_eq!(app.view, ViewMode::Notes);
    }

    #[test]
    fn tick_from_a_pasted_list() {
        let st = |t: &str| Step { text: t.into(), ..Default::default() };
        let steps = vec![st("The Traveller"), st("Shake on It"), st("#14 Pagan's Dilemma"), st("Ending #1: True Ending"), st("Money is Power")];
        // a PSNProfiles-style paste: names mixed with descriptions, percentages and rarity
        let paste = "The Traveller\nAcquired all trophies.\n19.18%\nRare\nPlatinum\n\tSHAKE ON IT!\nPagan\u{2019}s Dilemma\ntrue ending\n";
        assert_eq!(match_names(&steps, paste), vec![0, 1, 2, 3]);
        assert!(match_names(&steps, "Money").is_empty(), "only whole names match");
    }

    #[test]
    fn tick_before_and_untick_from() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let g = &mut app.lib.games[0].guides[0];
        g.steps.iter_mut().for_each(|s| s.done = false);
        g.steps[2].missed = true;
        g.tick_before(5);
        let done: Vec<bool> = g.steps.iter().take(7).map(|s| s.done).collect();
        assert_eq!(done, [true, true, true, true, true, false, false]);
        assert!(!g.steps[2].missed, "ticking clears a manual 'missed'");
        g.untick_from(3);
        assert_eq!(g.steps.iter().filter(|s| s.done).count(), 3);
        g.tick_before(9999); // past the end: everything
        assert!(g.steps.iter().all(|s| s.done));
    }

    #[test]
    fn search_everything_finds_items_anywhere() {
        let ctx = egui::Context::default();
        let app = app(&ctx);
        let hits = search_library(&app.lib, "objective 3", 50);
        assert!(hits.iter().any(|h| h.title.starts_with("Objective 3:") && h.item == Some(3)), "{hits:#?}");
        let hits = search_library(&app.lib, "fire emblem", 50);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].game, hits[0].guide), (4, None));
        // matches inside How text come with a snippet
        let hits = search_library(&app.lib, "explanation of how", 3);
        assert_eq!(hits.len(), 3, "limit respected");
        assert!(hits[0].snippet.contains("explanation of how"));
    }

    #[test]
    fn arrow_keys_move_and_space_ticks() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 850.0);
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        for _ in 0..2 {
            run(&mut app, &ctx, size, vec![]);
        }
        // first press lands on the next item to do (item 12 in the sample), then moves down
        run(&mut app, &ctx, size, vec![key(Key::ArrowDown)]);
        let first = app.list_cursor.expect("highlight").1;
        run(&mut app, &ctx, size, vec![key(Key::ArrowDown)]);
        let second = app.list_cursor.unwrap().1;
        assert!(second > first, "{first} -> {second}");
        let was = app.lib.games[0].guides[0].steps[second].done;
        run(&mut app, &ctx, size, vec![key(Key::Space)]);
        assert_eq!(app.lib.games[0].guides[0].steps[second].done, !was, "Space toggles the highlighted item");
        run(&mut app, &ctx, size, vec![key(Key::ArrowUp)]);
        assert_eq!(app.list_cursor.unwrap().1, first);
    }

    #[test]
    fn synced_achievements_tick_or_make_a_checklist() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let a = |n: &str, got: bool| accounts::Achievement { name: n.into(), description: format!("About {n}"), achieved: got, rarity: if got { Some(50.0) } else { Some(2.5) }, id: String::new() };
        // the sample game has no achievement items: a checklist is made from the platform's list
        let msg = app.apply_achievements(1, Source::Steam, &[a("The Body", true), a("Our Song", false)]);
        assert!(msg.contains("made an achievement checklist with 2 items, 1 already earned"), "{msg}");
        let g = &app.lib.games[0];
        let gi = g.trophy_guide().unwrap();
        assert_eq!(g.guides[gi].steps.iter().map(|s| (s.text.as_str(), s.done)).collect::<Vec<_>>(), [("The Body", true), ("Our Song", false)]);
        // syncing again ticks matches in that checklist, ignoring case and punctuation
        let msg = app.apply_achievements(1, Source::Xbox, &[a("OUR SONG!", true), a("Unknown One", true)]);
        assert!(msg.contains("ticked 1 new") && msg.contains("1 Xbox names didn't match"), "{msg}");
        assert!(app.lib.games[0].guides[gi].steps.iter().all(|s| s.done));
        assert_eq!(app.lib.games[0].guides[gi].steps[0].rarity, Some(50.0), "rarity comes along");
        // the dashboard and the sync window draw
        app.trophies_open = true;
        run(&mut app, &ctx, Vec2::new(1400.0, 850.0), vec![]);
        app.trophies_open = false;
        app.sync_window = Some(1);
        run(&mut app, &ctx, Vec2::new(1400.0, 850.0), vec![]);
    }

    #[test]
    fn cover_crop_fills_the_case() {
        // a wide image is cropped left and right, a tall one top and bottom
        let wide = icons::cover_uv(Vec2::new(400.0, 200.0), Vec2::new(50.0, 70.0));
        assert!(wide.min.y == 0.0 && wide.max.y == 1.0 && wide.min.x > 0.0 && (wide.min.x + wide.max.x - 1.0).abs() < 1e-5);
        let tall = icons::cover_uv(Vec2::new(100.0, 400.0), Vec2::new(50.0, 70.0));
        assert!(tall.min.x == 0.0 && tall.max.x == 1.0 && tall.min.y > 0.0);
        // a game with "No cover" never asks for one
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.lib.games[0].cover = "none".into();
        app.want_cover(1);
        assert!(matches!(app.covers.get(&1), Some(Fav::Missing)));
    }

    #[test]
    fn input_row_stays_on_its_line() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut buf = String::new();
        let mut got = None;
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 900.0))), ..Default::default() };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let top = ui.cursor().top();
                let (r, _, _) = input_row(ui, &mut buf, "Search…", "t", "+ Add");
                let after = ui.cursor().top();
                ui.label("next line");
                got = Some((top, r.rect, after));
            });
        });
        let (top, rect, after) = got.unwrap();
        assert!(rect.top() - top < 8.0, "text box at y={} instead of near the top ({top})", rect.top());
        assert!(after - top < 40.0, "the row took {}px of height instead of one line", after - top);
        assert!(rect.width() > 250.0, "text box only {}px wide in a 400px panel", rect.width());
    }

    #[test]
    fn side_panels_stay_where_you_put_them() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 850.0);
        for _ in 0..5 {
            run(&mut app, &ctx, size, vec![]);
        }
        let (left, right) = (width(&ctx, "games"), width(&ctx, "journal"));

        // left alone, nothing creeps
        for _ in 0..30 {
            run(&mut app, &ctx, size, vec![]);
        }
        assert_eq!((width(&ctx, "games"), width(&ctx, "journal")), (left, right), "panels changed width on their own");

        // drag the Games panel's edge 70px left: it stays there
        drag(&mut app, &ctx, size, Pos2::new(left, 400.0), Pos2::new(left - 70.0, 400.0));
        for _ in 0..10 {
            run(&mut app, &ctx, size, vec![]);
        }
        let want = left - 70.0;
        assert!((width(&ctx, "games") - want).abs() <= 2.0, "Games panel snapped to {} after dragging to {want}", width(&ctx, "games"));

        // drag the journal panel's edge 70px right: it stays there
        let edge = size.x - right;
        drag(&mut app, &ctx, size, Pos2::new(edge, 400.0), Pos2::new(edge + 70.0, 400.0));
        for _ in 0..10 {
            run(&mut app, &ctx, size, vec![]);
        }
        let want = right - 70.0;
        assert!((width(&ctx, "journal") - want).abs() <= 2.0, "journal panel snapped to {} after dragging to {want}", width(&ctx, "journal"));

        // squeeze the Games panel to its minimum: it stops there and its contents still fit
        let now = width(&ctx, "games");
        drag(&mut app, &ctx, size, Pos2::new(now, 400.0), Pos2::new(60.0, 400.0));
        for _ in 0..10 {
            run(&mut app, &ctx, size, vec![]);
        }
        assert!((width(&ctx, "games") - 220.0).abs() <= 1.0, "Games panel at {} instead of its 220 minimum", width(&ctx, "games"));

        // shrinking the whole window doesn't make the panels jump around
        let small = Vec2::new(1000.0, 700.0);
        for _ in 0..10 {
            run(&mut app, &ctx, small, vec![]);
        }
        let settled = (width(&ctx, "games"), width(&ctx, "journal"));
        for _ in 0..20 {
            run(&mut app, &ctx, small, vec![]);
        }
        assert_eq!((width(&ctx, "games"), width(&ctx, "journal")), settled, "panels kept moving after the window was resized");

        // and nothing drawn in a side panel is wider than the panel (it would be cut off)
        let over = OVERFLOW.with(|o| o.get());
        assert!(over <= 0.5, "side panel content is {over:.1}px wider than its panel");
    }

    /// Every piece of text drawn in a frame, one per line.
    fn drawn_text(app: &mut App, ctx: &egui::Context, size: Vec2) -> String {
        fn walk(s: &egui::Shape, out: &mut String) {
            match s {
                egui::Shape::Text(t) => {
                    out.push_str(t.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(v) => v.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
        let out = ctx.run(input, |ctx| app.frame(ctx));
        let mut text = String::new();
        for c in &out.shapes {
            walk(&c.shape, &mut text);
        }
        text
    }

    #[test]
    fn hotkey_ticks_the_next_item_and_the_mini_window_shows_it() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 850.0);
        assert_eq!(app.current_list(), Some((1, 2)));
        let next = app.lib.games[0].guides[0].next_index(Some((4, 2))).unwrap();
        let name = app.lib.games[0].guides[0].steps[next].text.clone();
        app.mini_open = true;
        let text = drawn_text(&mut app, &ctx, size);
        assert!(text.contains(&name), "the mini window shows the next item");
        assert_eq!(app.tick_next().as_deref(), Some(name.as_str()));
        assert!(app.lib.games[0].guides[0].steps[next].done);
        app.untick_last();
        assert!(!app.lib.games[0].guides[0].steps[next].done, "the undo hotkey unticks it");
        // closing the (embedded) mini window turns it off; drawing it a few times is fine
        for _ in 0..3 {
            run(&mut app, &ctx, size, vec![]);
        }
        assert!(app.mini_open);
    }

    #[test]
    fn play_timer_runs_for_one_game_at_a_time() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.start_playing(1);
        assert!(app.lib.games[0].session_start.is_some());
        app.lib.games[0].session_start = Some(model::now() - 600);
        app.start_playing(4);
        assert!(app.lib.games[0].session_start.is_none(), "starting another game stops the first");
        assert!(app.lib.games[0].play_secs >= 600);
        assert!(app.lib.games[0].log.last().unwrap().text.starts_with("Played 10 min"));
        assert_eq!(app.playing().map(|g| g.id), Some(4));
        let text = drawn_text(&mut app, &ctx, Vec2::new(1400.0, 850.0));
        assert!(text.contains("0:00:0"), "the taskbar shows the running timer");
        app.stop_playing(4);
        assert!(app.playing().is_none());
    }

    #[test]
    fn spoiler_guard_covers_what_comes_after_your_place() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 1600.0);
        app.view = ViewMode::Checklist;
        app.hide_done = true;
        let next = app.lib.games[0].guides[0].next_index(Some((4, 2))).unwrap();
        let later = app.lib.games[0].guides[0].steps[next + 1].text.clone();
        let text = drawn_text(&mut app, &ctx, size);
        assert!(text.contains(&later));
        app.lib.games[0].spoiler_guard = true;
        let text = drawn_text(&mut app, &ctx, size);
        assert!(!text.contains(&later), "later items are covered");
        assert!(text.contains("hidden by the spoiler guard"));
        assert!(text.contains(&app.lib.games[0].guides[0].steps[next].text), "the next item isn't");
        app.revealed.insert((2, next + 1));
        assert!(drawn_text(&mut app, &ctx, size).contains(&later), "a clicked one shows");
    }

    #[test]
    fn notes_images_and_rarity_show_on_items() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1400.0, 1600.0);
        app.view = ViewMode::Checklist;
        let s = &mut app.lib.games[0].guides[0].steps[0];
        s.note = "Came back after chapter 5".into();
        s.image = "Z:/nowhere/screenshot.png".into();
        s.rarity = Some(3.25);
        let text = drawn_text(&mut app, &ctx, size);
        assert!(text.contains("Note: Came back after chapter 5"));
        assert!(text.contains("Image not found"));
        assert!(text.contains("3.2%"));
        // Find searches notes
        assert!(search_library(&app.lib, "chapter 5", 10).iter().any(|h| h.item == Some(0)));
        // a real image loads as a thumbnail
        let dir = std::env::temp_dir().join(format!("jagt-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("shot.png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 40, 90, 255])).save(&png).unwrap();
        app.lib.games[0].guides[0].steps[0].image = png.display().to_string();
        let text = drawn_text(&mut app, &ctx, size);
        assert!(!text.contains("Image not found"));
        assert!(matches!(app.thumbs.get(&png.display().to_string()), Some(Fav::Ready(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missables_dashboard_lists_what_you_could_miss() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.missables_open = true;
        let text = drawn_text(&mut app, &ctx, Vec2::new(1400.0, 1200.0));
        // sample items with a deadline: every third one; those under 12 are done
        assert!(text.contains("Objective 12: A rather long"), "{text}");
        assert!(!text.contains("Objective 3: A rather long"), "done items aren't listed");
        assert!(text.contains("Ends 9/25"));
        assert!(text.contains("things you could still miss"));
    }

    #[test]
    fn tags_filter_the_games_list() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        assert_eq!(app.sorted_ids().len(), 2);
        app.tag_filter = Some("co-op".into());
        assert_eq!(app.sorted_ids(), vec![1]);
        app.tag_filter = None;
        app.search = "co-op".into();
        assert_eq!(app.sorted_ids(), vec![1], "search finds tags too");
        app.search.clear();
        let text = drawn_text(&mut app, &ctx, Vec2::new(1400.0, 850.0));
        assert!(text.contains("#co-op"));
    }

    #[test]
    fn reader_shows_the_guide_text_at_your_place() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let size = Vec2::new(1500.0, 900.0);
        let _ = std::fs::remove_dir_all(&app.pages_dir);
        let mut page = String::from("## Intro\nSome words about the game.\n\n");
        for a in 0..4 {
            page.push_str(&format!("## Area {a} (cleared data, part 2)\n"));
            for i in a * 10..a * 10 + 10 {
                page.push_str(&format!("Objective {i}: A rather long objective name that keeps going and going\nDo the thing.\n\n"));
            }
        }
        model::save_page(&app.pages_dir, 2, &page).unwrap();
        app.view = ViewMode::Checklist;
        app.reader_open = true;
        let next = app.lib.games[0].guides[0].next_index(Some((4, 2))).unwrap();
        run(&mut app, &ctx, size, vec![]);
        let text = drawn_text(&mut app, &ctx, size);
        let r = app.reader.as_ref().expect("the text loaded");
        let t = r.target.expect("found the item in the text");
        assert!(r.lines[t].starts_with(&format!("Objective {next}:")), "{}", r.lines[t]);
        assert!(text.contains("Guide text"));
        // the reader scrolled there, so the line is drawn
        assert!(text.contains(&format!("Objective {next}: A rather")));
        // search
        app.reader_find = "area 3".into();
        run(&mut app, &ctx, size, vec![]);
        assert_eq!(app.reader.as_ref().unwrap().found.len(), 1);
        let _ = std::fs::remove_dir_all(&app.pages_dir);
    }

    #[test]
    fn finding_an_item_in_guide_text() {
        let lines: Vec<String> = ["## Castle Town", "Chest 1 - Potion", "## Forest", "Chest 1 - Ether", "Objective 14: Frozen Rail Ride"].iter().map(|s| s.to_string()).collect();
        assert_eq!(find_line(&lines, "Forest", "Chest 1", ""), Some(3), "the one in the item's area");
        assert_eq!(find_line(&lines, "", "Objective 14: Frozen Rail Ride", ""), Some(4));
        assert_eq!(find_line(&lines, "Castle Town", "Achievement: Frozen Rail Ride", ""), Some(4), "the name after a prefix");
        assert_eq!(find_line(&lines, "", "Nowhere", ""), None);
    }

    #[test]
    fn new_run_window_draws_and_starts_a_run() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.run_window = Some((1, "NG+".into(), HashSet::new(), true));
        run(&mut app, &ctx, Vec2::new(1400.0, 850.0), vec![]);
        assert!(app.run_window.is_some());
        app.lib.games[0].new_run("NG+", &[], true, model::now());
        let text = drawn_text(&mut app, &ctx, Vec2::new(1400.0, 850.0));
        assert!(text.contains("NG+"), "the Run menu shows the new run");
        assert!(app.lib.games[0].guides[0].steps.iter().all(|s| !s.done));
    }
}

#[cfg(test)]
mod reader_check {
    /// `JAGT_PAGE=page.txt cargo test reader_finds_items -- --ignored --nocapture`: how many of a
    /// real page's checklist items the reader can find in its text.
    #[test]
    #[ignore]
    fn reader_finds_items() {
        let Ok(path) = std::env::var("JAGT_PAGE") else { return };
        for p in path.split(';') {
            let text = std::fs::read_to_string(p).unwrap();
            let r = super::Reader::new(1, &text);
            let steps = crate::checklist::parse(&text);
            let mut miss = Vec::new();
            let mut order_ok = 0;
            let mut last = 0;
            for s in &steps {
                match super::find_line(&r.lines, &s.section, &s.text, &s.anchor) {
                    Some(l) => {
                        if l >= last {
                            order_ok += 1;
                        }
                        last = l;
                    }
                    None => miss.push(s.text.clone()),
                }
            }
            println!("{p}: {} items, {} found, {} in page order; missed {:?}", steps.len(), steps.len() - miss.len(), order_ok, miss.iter().take(5).collect::<Vec<_>>());
        }
    }
}
