use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

use egui::{Align, Align2, Color32, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};

use crate::icons;
use crate::links;
use crate::checklist;
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
    /// Reopen the explorer where it was left.
    explorer_open: bool,
    ex_path: Vec<String>,
    window_size: Option<[f32; 2]>,
    maximized: bool,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { filter: None, selected: None, online: true, glow: true, animate: true, folder_view: true, closed_folders: Vec::new(), ex_layout: ExplorerLayout::Icons, compact_rows: false, view: ViewMode::All, explorer_open: false, ex_path: Vec::new(), window_size: None, maximized: false }
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
}

/// What a checklist row asks for.
enum RowAct {
    Toggle,
    /// Open or close a compact row's details.
    Details,
    Missed,
    Optional,
    Find,
    Delete,
    Up,
    Down,
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
            open_rows: HashSet::new(),
            explorer_open: saved.explorer_open,
            ex_path: saved.ex_path,
            ex_back: Vec::new(),
            ex_fwd: Vec::new(),
            ex_sel: None,
            row_heights: HashMap::new(),
            online: Online::new(ctx.clone()),
            favicons: HashMap::new(),
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
        };
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
            .filter(|g| q.is_empty() || g.title.to_lowercase().contains(&q) || g.platform.to_lowercase().contains(&q))
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
            Some(id) => self.open_guide(gid, id),
            None => self.set_status("Add a guide to this game first"),
        }
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
                Done::PageText { guide, text } => {
                    // only if the paste window is still open for that guide
                    if let Some((gid, buf)) = &mut self.bulk {
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
            }
        }
        any
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
        let Some(g) = self.lib.game_mut(gid) else { return };
        ui.add(egui::TextEdit::singleline(&mut g.title).font(bold(24.0)).desired_width(f32::INFINITY).frame(false).hint_text("Game title"));
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
            info.push(format!("added {}", model::date(g.added)));
            info.push(match g.last_played {
                Some(t) => format!("last played {}", model::ago(t)),
                None => "not played yet".into(),
            });
            ui.label(RichText::new(info.join(" · ")).small().color(DIM));
        });
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

        // ---- toolbar: which guide, view switch and actions
        let mut paste = false;
        let mut untick = false;
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

        let compact = self.compact_rows;
        let App { lib, list_kind, list_scope, hide_done, list_search, list_section, left_view, collapsed, scroll_to_next, step_bufs, row_heights, open_rows, .. } = self;
        let Some(game) = lib.game_mut(gid) else { return };
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
                    let print = row_fingerprint(s, states[i], is_next, can_find, w, compact, open);
                    if let Some(&(fp, h)) = row_heights.get(&(aid, i)) {
                        let r = Rect::from_min_size(ui.cursor().min, Vec2::new(w, h));
                        if fp == print && !(is_next && scroll_next) && !ui.is_rect_visible(r) {
                            ui.allocate_space(Vec2::new(w, h));
                            ui.add_space(4.0);
                            continue;
                        }
                    }
                    let resp = step_row(ui, s, i, states[i], is_next, can_find, compact, open, &mut acts);
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
                RowAct::Up if i > 0 => guide.steps.swap(i, i - 1),
                RowAct::Down if i + 1 < guide.steps.len() => guide.steps.swap(i, i + 1),
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
        }
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
                        for (t, cover) in tiles.iter().zip(&covers) {
                            let item = ExplorerItem::Game(t.id);
                            let r = game_tile(ui, t, *cover, sel.as_ref() == Some(&item));
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
            if resp.on_hover_text("Reopen the last guide at your saved spot (Enter)").clicked() && enabled {
                go = true;
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
                if ui.checkbox(&mut self.online_on, "Look up guide titles and site icons").changed() && self.online_on {
                    self.favicons.retain(|_, f| !matches!(f, Fav::Missing));
                }
                ui.label(RichText::new("Titles come from the page itself (YouTube via oEmbed). Icons come from Google's favicon service and are cached.").small().color(DIM));
                if ui.button("Clear icon cache").clicked() {
                    online::clear_cache();
                    self.favicons.clear();
                    self.set_status("Icon cache cleared");
                }

                heading(ui, "Your data");
                ui.label(RichText::new(self.lib_path.display().to_string()).small().color(YELLOW));
                ui.label(
                    RichText::new("Saved automatically. The previous version is kept as library.bak.json. Put a jagt-data folder next to the .exe for portable mode.")
                        .small()
                        .color(DIM),
                );
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
        let pumped = self.pump(ctx);
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
        if let Some(gid) = gid.filter(|_| !self.explorer_open && view == ViewMode::All) {
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
            Some(gid) if view != ViewMode::All => {
                // one window gets all the space
                let (_, b) = match view {
                    ViewMode::Game => window_ctl(ui, "Game", true, |ui| {
                        self.game_header(ui, gid);
                        ui.allocate_space(ui.available_size());
                    }),
                    ViewMode::Guides => window_ctl(ui, "Guides", true, |ui| self.guides_view(ui, gid, f32::INFINITY)),
                    ViewMode::Checklist => window_ctl(ui, "Checklist", true, |ui| self.checklist_view(ui, gid)),
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
                    if let (_, Some(b)) = window_ctl(ui, "Checklist", false, |ui| self.checklist_view(ui, gid)) {
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
        self.settings_window(ctx);
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
            explorer_open: self.explorer_open,
            ex_path: self.ex_path.clone(),
            window_size: self.window_size.map(|s| [s.x, s.y]),
            maximized: self.maximized,
        };
        eframe::set_value(storage, "jagt", &saved);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
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
                        } else if ui.button("Look up title again").clicked() {
                            acts.push(CardAct::Refetch);
                            ui.close_menu();
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

fn game_tile(ui: &mut Ui, t: &GameTile, cover: Option<egui::TextureId>, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(TILE, Sense::click());
    let p = ui.painter();
    tile_bg(p, rect, selected, resp.hovered());
    let case = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 46.0), Vec2::new(56.0, 74.0));
    icons::game_case(p, case, platform_color(&t.platform), cover, t.status.color());
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
#[allow(clippy::too_many_arguments)]
fn row_fingerprint(s: &Step, state: StepState, is_next: bool, can_find: bool, width: f32, compact: bool, open: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&s.text, &s.detail, &s.how, &s.reward, &s.deadline, s.optional, s.hidden).hash(&mut h);
    (state as u8, is_next, can_find, width.round() as i32, compact, open).hash(&mut h);
    h.finish()
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
/// Clicking anywhere on the row ticks it.
#[allow(clippy::too_many_arguments)]
fn step_row(ui: &mut Ui, s: &Step, i: usize, state: StepState, is_next: bool, can_find: bool, compact: bool, open: bool, acts: &mut Vec<(usize, RowAct)>) -> egui::Response {
    let has_details = !s.detail.is_empty() || !s.how.is_empty();
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
            let stroke = if is_next {
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
                                if can_find && ui.small_button("Find in guide").on_hover_text("Open the guide scrolled to this item").clicked() {
                                    acts.push((i, RowAct::Find));
                                }
                                if !s.reward.is_empty() {
                                    tag(ui, &format!("Reward: {}", s.reward), if faded { DIM } else { TEAL });
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
                        if show_details && !s.detail.is_empty() {
                            ui.add(egui::Label::new(RichText::new(&s.detail).color(if faded { DIM } else { YELLOW })).wrap().selectable(false));
                        }
                        if show_details && !s.how.is_empty() {
                            ui.add_space(2.0);
                            let col = if faded { DIM.gamma_multiply(0.7) } else { DIM };
                            ui.add(egui::Label::new(RichText::new(format!("How: {}", s.how)).color(col)).wrap().selectable(false));
                        }
                    });
                });
            });
        })
        .response;
    let resp = if compact && !open && has_details {
        let mut tip = s.detail.clone();
        if !s.how.is_empty() {
            if !tip.is_empty() {
                tip.push_str("\n\n");
            }
            tip.push_str(&format!("How: {}", s.how));
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
        acts.push((i, RowAct::Toggle));
    }
    resp.context_menu(|ui| {
        if ui.button(if done { "Untick" } else { "Tick off" }).clicked() {
            acts.push((i, RowAct::Toggle));
            ui.close_menu();
        }
        let missed_label = if s.missed { "Not missed after all" } else { "Mark as missed / skipped" };
        if !s.missed && missed && !s.deadline.is_empty() {
            ui.label(RichText::new(format!("Past its deadline ({})", s.deadline)).small().color(DIM));
        } else if ui.button(missed_label).clicked() {
            acts.push((i, RowAct::Missed));
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
        let mut frame = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
            let out = ctx.run(input, |ctx| app.frame(ctx));
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
        };
        for _ in 0..20 {
            frame(&mut app, vec![]);
        }
        let mut time = |label: &str, app: &mut App, make: &dyn Fn(usize) -> Vec<egui::Event>| {
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
}
