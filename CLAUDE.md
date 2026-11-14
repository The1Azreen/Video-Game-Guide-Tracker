# JAGT — notes for working on this repo

Rust + egui 0.31 desktop app. UI in `src/app.rs`, data model in `src/model.rs`, checklist parsing in
`src/checklist.rs`, link handling in `src/links.rs`, fetching in `src/online.rs`, look in `src/theme.rs`
(shared with the JAMP music player).

## Layout rules (these have caused "windows moving on their own" bugs before)

- A resizable `SidePanel` remembers its width as the width of whatever was drawn in it. Content that is
  even 1px too wide makes the panel creep wider every frame and snap back when the user drags it.
  - Side panel contents always go through `contained(ui, ...)`.
  - A text box with a button next to it: use `input_row(...)`, never `desired_width(available_width() - N)`.
  - Never put a bare `with_layout(...)` in a top-down `Ui`: it takes all the height left and centres its
    contents in it (rows end up mid-panel). Give it a size with `allocate_ui_with_layout`, or use `horizontal`.
  - Rows of buttons in a side panel: `horizontal_wrapped`, so they wrap when the panel is narrow.
- Lists must not reorder themselves while being used (the games list is sorted by status, then title).
- Ticked checklist items keep their height; "Hide done" removes them instead.
- egui memory (scroll positions) is not persisted: `persist_egui_memory` returns false.

## Performance

- Checklist rows off screen are skipped using `row_heights` (keyed by guide + item, with a fingerprint of
  everything that affects height: see `row_fingerprint`). Anything new that changes a row's height must go
  into the fingerprint.
- Idle redraws: ~15 fps only while the stars animate (focused window, animation on); otherwise 1 s.
- The unsaved-changes check serialises the whole library; it only runs on frames with real input.
- Benchmark on a copy of a real library:
  `JAGT_BENCH_LIB=path/to/copy.json cargo test --release bench_frames -- --ignored --nocapture`

`cargo test` includes `app::tests::side_panels_stay_where_you_put_them`, which drives the real UI headless:
it drags both panel edges, resizes the window, and fails if a panel moves on its own or if anything in a
side panel is wider than the panel. `input_row_stays_on_its_line` checks rows keep their place. Run
`cargo test` after any UI change.

The exe has no console in any build (`windows_subsystem = "windows"`); `--help`, `--parse-checklist` and
`--html-to-text` attach to the terminal's console to print. The user runs `target\release\jagt.exe`, so
rebuild with `cargo build --release` after changes.

## Data

The user's library is in `jagt-data/` (gitignored) when run from this repo. Back it up before editing it by
script, and only write it while the app is closed (`tasklist | grep -i jagt`). Never launch the app on the
user's screen to test; use the headless test, or a copy of the exe in a scratch folder with its own `jagt-data/`.
