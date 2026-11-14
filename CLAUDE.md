# JAGT — notes for working on this repo

Rust + egui 0.31 desktop app. UI in `src/app.rs`, data model in `src/model.rs`, checklist parsing in
`src/checklist.rs`, link handling in `src/links.rs`, fetching in `src/online.rs`, look in `src/theme.rs`
(shared with the JAMP music player).

## Layout rules (these have caused "windows moving on their own" bugs before)

- A resizable `SidePanel` remembers its width as the width of whatever was drawn in it. Content that is
  even 1px too wide makes the panel creep wider every frame and snap back when the user drags it.
  - Side panel contents always go through `contained(ui, ...)`.
  - A text box with a button next to it: use `input_row(...)`, never `desired_width(available_width() - N)`.
  - Rows of buttons in a side panel: `horizontal_wrapped`, so they wrap when the panel is narrow.
- Lists must not reorder themselves while being used (the games list is sorted by status, then title).
- Ticked checklist items keep their height; "Hide done" removes them instead.
- egui memory (scroll positions) is not persisted: `persist_egui_memory` returns false.

`cargo test` includes `app::tests::side_panels_stay_where_you_put_them`, which drives the real UI headless:
it drags both panel edges, resizes the window, and fails if a panel moves on its own or if anything in a
side panel is wider than the panel. Run it after any UI change.

## Data

The user's library is in `jagt-data/` (gitignored) when run from this repo. Back it up before editing it by
script, and only write it while the app is closed (`tasklist | grep -i jagt`). Never launch the app on the
user's screen to test; use the headless test, or a copy of the exe in a scratch folder with its own `jagt-data/`.
