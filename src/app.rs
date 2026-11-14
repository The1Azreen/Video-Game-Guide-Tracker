use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

use egui::{Align, Align2, Color32, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};

use crate::icons;
use crate::links;
use crate::checklist;
use crate::model::{self, Game, Guide, Kind, Library, LogEntry, Status, Step, StepKind, StepState};
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
    window_size: Option<[f32; 2]>,
    maximized: bool,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { filter: None, selected: None, online: true, glow: true, window_size: None, maximized: false }
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

/// What a guide card asks the app to do once the list has been drawn.
enum CardAct {
    Open,
    Up,
    Down,
    Remove,
    Refetch,
    BulkSteps,
    ShowChecklist,
}

/// What a checklist row asks for.
enum RowAct {
    Toggle,
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
        set_glow(saved.glow);

        let lib_path = model::library_path();
        let (lib, problem) = model::load(&lib_path);
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
            online: Online::new(cc.egui_ctx.clone()),
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
            show_settings: opts.open_settings,
            status: None,
            started: Instant::now(),
            window_size: saved.window_size.map(Vec2::from),
            maximized: saved.maximized,
            restore_window: Some((saved.window_size, saved.maximized)),
            restore_wait: 0,
            persist: !opts.fresh,
        };
        app.file_mtime = mtime(&app.lib_path);
        app.snapshot = app.lib.clone();
        if app.selected.is_none_or(|id| !app.lib.games.iter().any(|g| g.id == id)) {
            app.selected = app.sorted_ids().first().copied();
        }
        if let Some(msg) = problem {
            app.set_status_for(msg, 12.0);
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
        if self.online_on && known_title.is_none() {
            self.online.send(Job::Title { game: game_id, guide: id, url });
        }
        self.set_status(format!("Added {}", kind.label().to_lowercase()));
        true
    }

    fn open_guide(&mut self, game_id: u64, guide_id: u64) {
        let Some(game) = self.lib.game_mut(game_id) else { return };
        let Some(g) = game.guides.iter_mut().find(|g| g.id == guide_id) else { return };
        let url = g.resume_url();
        g.last_opened = Some(model::now());
        let spot = g.spot();
        game.last_guide = Some(guide_id);
        game.touch();
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
                if ui.button("+ Add game").on_hover_text("Ctrl+N").clicked() {
                    self.add_game = Some(NewGame { title: String::new(), platform: String::new(), status: Status::Playing, focus: true });
                }
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Games list (left)

    fn games_view(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search games…").desired_width(ui.available_width() - 64.0));
            if ui.button("+ Add").clicked() {
                self.add_game = Some(NewGame { title: String::new(), platform: String::new(), status: self.filter.unwrap_or(Status::Playing), focus: true });
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
        let mut set_status: Option<(u64, Status)> = None;
        egui::ScrollArea::vertical().id_salt("games").auto_shrink([false, false]).show(ui, |ui| {
            for (n, id) in ids.iter().enumerate() {
                let Some(g) = self.lib.games.iter().find(|g| g.id == *id) else { continue };
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, GAME_ROW_H), Sense::click());
                let p = ui.painter();
                let sel = self.selected == Some(g.id);
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
                    self.selected = Some(g.id);
                }
                resp.context_menu(|ui| {
                    ui.label(RichText::new("Set status").color(TEAL));
                    for s in Status::ALL {
                        if ui.selectable_label(g.status == s, s.label()).clicked() {
                            set_status = Some((g.id, s));
                            ui.close_menu();
                        }
                    }
                });
            }
        });
        if let Some((id, s)) = set_status {
            if let Some(g) = self.lib.game_mut(id) {
                g.status = s;
            }
        }
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
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.new_url)
                    .hint_text("Paste a guide link: website, spreadsheet, YouTube…")
                    .desired_width(ui.available_width() - 100.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button("+ Add guide").clicked() || enter) && !self.new_url.trim().is_empty() {
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
        });
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
                    if ui.add(egui::Button::new("Full list").selected(!self.left_view)).clicked() {
                        self.left_view = false;
                    }
                    if ui.add(egui::Button::new("What's left").selected(self.left_view)).on_hover_text("What's still to do, area by area").clicked() {
                        self.left_view = true;
                    }
                }
            });
        });

        let App { lib, list_kind, list_scope, hide_done, list_search, list_section, left_view, collapsed, scroll_to_next, step_bufs, .. } = self;
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
            let web = guide.kind != Kind::Video && guide.kind != Kind::Sheet;
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
                    let resp = step_row(ui, s, i, states[i], is_next, web && !s.anchor.is_empty(), &mut acts);
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
        ui.horizontal(|ui| {
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
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.new_log).hint_text("What happened this session?").desired_width(ui.available_width() - 50.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button("Add").clicked() || enter) && !self.new_log.trim().is_empty() {
                g.log.push(LogEntry { at: model::now(), text: std::mem::take(&mut self.new_log).trim().to_string() });
                g.touch();
            }
        });
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
            .map_or((String::new(), 0, false), |g| (g.url.clone(), g.steps.len(), g.kind != Kind::Video && g.kind != Kind::Sheet));

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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.track_window(ctx);
        let now = self.time();
        let had_input = ctx.input(|i| !i.events.is_empty() || i.pointer.any_down());
        let pumped = self.pump(ctx);
        self.keyboard(ctx);

        paint_night_sky(&ctx.layer_painter(egui::LayerId::background()), ctx.screen_rect(), now);

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
            window(ui, "Games", |ui| self.games_view(ui));
        });

        let gid = self.selected.filter(|id| self.lib.games.iter().any(|g| g.id == *id));
        if let Some(gid) = gid {
            egui::SidePanel::right("journal").resizable(true).default_width(330.0).width_range(260.0..=520.0).frame(pad).show(ctx, |ui| {
                window(ui, "Where I Left Off", |ui| self.resume_view(ui, gid));
                ui.add_space(12.0);
                window(ui, "Session Log", |ui| self.log_view(ui, gid));
            });
        }

        egui::CentralPanel::default().frame(pad).show(ctx, |ui| match gid {
            Some(gid) => {
                window(ui, "Game", |ui| self.game_header(ui, gid));
                ui.add_space(12.0);
                let has_guides = self.lib.games.iter().any(|g| g.id == gid && !g.guides.is_empty());
                if has_guides {
                    let max_h = (ui.available_height() * 0.28).max(130.0);
                    window(ui, "Guides", |ui| self.guides_view(ui, gid, max_h));
                    ui.add_space(12.0);
                    window(ui, "Checklist", |ui| self.checklist_view(ui, gid));
                } else {
                    window(ui, "Guides", |ui| self.guides_view(ui, gid, f32::INFINITY));
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

        // A slow tick keeps the stars drifting and lets the autosave timer fire.
        ctx.request_repaint_after(std::time::Duration::from_millis(66));
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
    let compact = g.steps.is_empty() && !matches!(g.kind, Kind::Video | Kind::Sheet) && !c.expanded;
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
                        if ui.button("Look up title again").clicked() {
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
                let mut info = vec![g.kind.label().to_string(), links::host(&g.url)];
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
fn step_row(ui: &mut Ui, s: &Step, i: usize, state: StepState, is_next: bool, can_find: bool, acts: &mut Vec<(usize, RowAct)>) -> egui::Response {
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
            egui::Frame::new().fill(fill).stroke(stroke).corner_radius(3).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    if big_checkbox(ui, done).on_hover_text(if done { "Untick" } else { "Tick off" }).clicked() {
                        acts.push((i, RowAct::Toggle));
                    }
                    ui.add_space(4.0);
                    let (ir, _) = ui.allocate_exact_size(Vec2::new(22.0, 24.0), Sense::hover());
                    icons::step_icon(ui.painter(), Rect::from_center_size(ir.center(), Vec2::splat(20.0)), s.kind, faded);
                    ui.add_space(4.0);
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
                                    let mut t = RichText::new(&s.text).font(bold(18.0)).color(col);
                                    if done {
                                        t = t.strikethrough();
                                    }
                                    ui.add(egui::Label::new(t).wrap().selectable(false));
                                    if s.hidden {
                                        ui.label(RichText::new("hidden").small().color(DIM));
                                    }
                                });
                            });
                        });
                        // Finished items keep their full height (just dimmed) so nothing below
                        // them slides up under the mouse. "Hide done" removes them instead.
                        if !s.detail.is_empty() {
                            ui.add(egui::Label::new(RichText::new(&s.detail).color(if faded { DIM } else { YELLOW })).wrap().selectable(false));
                        }
                        if !s.how.is_empty() {
                            ui.add_space(2.0);
                            let col = if faded { DIM.gamma_multiply(0.7) } else { DIM };
                            ui.add(egui::Label::new(RichText::new(format!("How: {}", s.how)).color(col)).wrap().selectable(false));
                        }
                    });
                });
            });
        })
        .response;
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
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(buf).id_salt(("new-step", g.id)).desired_width(ui.available_width() - 70.0).hint_text("Add an item: chapter, collectible, trophy…"));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if (ui.button("+ Add").clicked() || enter) && !buf.trim().is_empty() {
            let section = g.steps.last().map(|s| s.section.clone()).unwrap_or_default();
            g.steps.push(Step { section, ..Step::task(checklist::plain_item(buf)) });
            buf.clear();
            if enter {
                r.request_focus();
            }
        }
    });
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
