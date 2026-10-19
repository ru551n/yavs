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
| `src/app.rs` | `App`: run state, DAP message handling (initialize, launch/attach, configurationDone, output, stopped, continued, exited, terminated), toolbar, status bar, `egui_dock` layout, console |
| `src/tests.rs` | Argument parsing; a scripted DAP server on a socket driven through the window with `egui_kittest` (Pause, Run, Stop, console, status); a connection failure; `launch_real_yavs` (ignored, needs `YAVS_BIN`) |

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
- The kittest accessibility tree finds widgets by their label: renaming a toolbar button breaks
  the tests on purpose.
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

Slices G2 to G6 (`docs/design/gui.md` 10) and the server slices S1 to S4 they need.
