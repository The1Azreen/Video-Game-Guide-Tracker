//! System-wide hotkeys, so the mini window can be used while a game has the focus (Windows only).

use std::sync::mpsc::Receiver;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HotAction {
    /// Tick the next item.
    TickNext,
    /// Untick what the hotkey last ticked.
    UndoTick,
    /// Show or hide the mini window.
    ToggleMini,
}

/// The keys, as shown to the user.
pub const KEYS: [(HotAction, &str, &str); 3] = [
    (HotAction::TickNext, "Ctrl+Alt+Space", "tick the next item"),
    (HotAction::UndoTick, "Ctrl+Alt+Backspace", "untick it again"),
    (HotAction::ToggleMini, "Ctrl+Alt+M", "show / hide the mini window"),
];

pub struct Hotkeys {
    #[cfg(windows)]
    _manager: global_hotkey::GlobalHotKeyManager,
    ids: Vec<(u32, HotAction)>,
    rx: Receiver<u32>,
    /// Keys another program already uses.
    pub taken: Vec<&'static str>,
}

impl Hotkeys {
    /// Registers the keys. Must be called on the thread that runs the window's event loop.
    #[cfg(windows)]
    pub fn new(ctx: egui::Context) -> Option<Self> {
        use global_hotkey::hotkey::{Code, HotKey, Modifiers};
        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
        let manager = GlobalHotKeyManager::new().ok()?;
        let mods = Some(Modifiers::CONTROL | Modifiers::ALT);
        let mut ids = Vec::new();
        let mut taken = Vec::new();
        for (action, label, _) in KEYS {
            let code = match action {
                HotAction::TickNext => Code::Space,
                HotAction::UndoTick => Code::Backspace,
                HotAction::ToggleMini => Code::KeyM,
            };
            let hk = HotKey::new(mods, code);
            match manager.register(hk) {
                Ok(()) => ids.push((hk.id(), action)),
                Err(_) => taken.push(label),
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                let _ = tx.send(e.id);
                ctx.request_repaint();
            }
        }));
        Some(Hotkeys { _manager: manager, ids, rx, taken })
    }

    #[cfg(not(windows))]
    pub fn new(_ctx: egui::Context) -> Option<Self> {
        None
    }

    /// The hotkeys pressed since the last call.
    pub fn poll(&self) -> Vec<HotAction> {
        let mut out = Vec::new();
        while let Ok(id) = self.rx.try_recv() {
            if let Some((_, a)) = self.ids.iter().find(|(i, _)| *i == id) {
                out.push(*a);
            }
        }
        out
    }
}
