# AGENTS.md: yavs-gui (the graphical debugger)

Read the root `AGENTS.md` first, then `docs/design/gui.md` (the binding design: process model,
panels, protocol, phased plan G1 to G6). This file is for an agent about to change `gui/`.

## 1. Purpose and boundaries

- `yavs-gui`: an eframe/egui window that drives `yavs run --dap=stdio` (a child it starts) or a
  `yavs run --dap=PORT` (`--attach HOST:PORT`) over the Debug Adapter Protocol.
- Its own Cargo workspace, **EUPL-1.2** (`LICENSE`), because libsurfer is. It depends on **no yavs
  crate**: the licence boundary is the process boundary (user, 2026-10-19). Never add a path
  dependency on `crates/*`; protocol facts come from `docs/cli-contract.md` 5 and
  `crates/yavs-dap/AGENTS.md`.
- The root workspace does not build it: root rule 13 checks do not cover `gui/`; its own are in
  section 5 and `.github/workflows/gui.yml`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/main.rs` | Command line (`[--yavs PATH] -- <yavs run arguments>`, `--attach HOST:PORT`, `--help`, `--version`), `eframe::run_native` |
| `src/dap.rs` | `Target`, `Session` (child or TCP, `request`, `Drop` = `disconnect` then kill after 2 s), reader threads into an `mpsc` channel, `Content-Length` framing |
| `src/app.rs` | `App` (dock) and `Dbg` (everything else): run state, DAP message handling, `Purpose` per request (answers of an earlier stop are dropped), `Action`s from the panels turned into requests, toolbar, status bar, `egui_dock` layout |
| `src/state.rs` | Plain structs, no window: `Breakpoints`, `Stack`, `VarTree` (lazy, paged), `parse_time`, `word_at`, `highlight`; unit tests |
| `src/panels.rs` | Console, Source (gutter, highlight, hover), Call stack, Locals, Watch, Breakpoints, the breakpoint dialog; they read `Dbg` and return `Action`s |
| `src/tests.rs` | Argument parsing; scripted DAP servers on a socket driven through the window with `egui_kittest` (G1 run controls; G2 stop, gutter, dialog, expansion, paging, hover, watch, setVariable, step); a connection failure; `launch_real_yavs` and `debug_real_yavs` (ignored, need `YAVS_BIN`) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| libsurfer is a git dependency pinned by `rev`, `default-features = false`; eframe/egui at libsurfer's version | Not on crates.io; one egui across the dependency tree | user, 2026-10-19; `docs/decisions.md` "GUI (M10)" |
| `egui_dock` for docking | Tabs, drag to dock, saved layouts | `docs/design/gui.md` 12 |
| The arguments after `--` are those of `yavs run` (the GUI adds `run` and `--dap=stdio`) | `yavs run --gui` forwards its own arguments unchanged | `docs/cli-contract.md` 6 |
| `yavs` is `--yavs`, else next to `yavs-gui`, else PATH; not found is an error naming the three places | No guessing | `docs/design/gui.md` 2 |
| The UI thread owns all state; reader threads only parse and send (`Incoming`) and wake the UI | No async runtime of our own | `docs/design/gui.md` 2 |
| Placeholder tabs name the slice that fills them | Visible progress, no fake panels | `docs/design/gui.md` 10 |
| Windows: console subsystem (a console opens with the window) | `--help` and errors stay visible; revisit with the release work (G6) | `ponytail:` until G6 |

## 4. Invariants and pitfalls

- Tests are headless (no `wgpu`, no snapshots): they need no GPU or display. Keep image
  snapshots out (they need a GPU backend on every CI platform).
- `Harness::step` does not wait for the reader thread: use `wait_for` in `src/tests.rs`.
- The kittest accessibility tree finds widgets by their label: renaming a toolbar button or
  changing the gutter text (`panels::gutter`: marker, space, line number right-aligned in 4)
  breaks the tests on purpose. egui_dock's tab buttons are not in the tree (`App::focus` brings
  a tab forward), and a widget scrolled out of its panel cannot be clicked (use
  `click_accesskit` or `Dbg::act`).
- Requests that need a stop (`stackTrace`, `scopes`, `variables`, `evaluate`) fail with `the
  simulation is running` otherwise; the panels grey out while running and ask again at each stop.
- Linux builds need no development packages: winit and glutin load X11, Wayland and EGL/GLX at
  run time. At run time an OpenGL driver is needed (software rendering works).

## 5. How to work here

```sh
cd gui
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
YAVS_BIN=../target/debug/yavs cargo test --locked -- --ignored   # against a built yavs
cargo run -- -- --work=work:<dir> tb                             # needs a display
```

## 6. Open

Slices G3 to G6 (`docs/design/gui.md` 10) and the server slices S1 (#396), S2 (#397), S4 (#398)
they need. Not in G2: global step (`yavs/stepAny`, S1), exception breakpoints (S4), locals of
suspended processes (S4), keyboard shortcuts (G5), breakpoint hit counts so far.
