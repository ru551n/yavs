# yavs-gui

The graphical debugger for [yavs](https://github.com/ru551n/yavs). It is a window that drives
`yavs run` over the Debug Adapter Protocol: it starts the simulator as a child process (or
connects to one you started) and shows breakpoints, the call stack, variables and the run
output. It is early: see "What works today" for what is missing.

yavs-gui is licensed under the EUPL-1.2 (`LICENSE`), unlike the simulator (MIT OR Apache-2.0).
It links no yavs code: the two programs only talk over a pipe or a socket. The release archives
ship it next to `yavs`, with the licence as `LICENSE-EUPL-1.2-yavs-gui`, which applies to the
`yavs-gui` binary only.

## Starting it

The usual way is a flag of `yavs run`; every other argument is passed on unchanged:

```
yavs analyze --work=lib:out/lib tb.vhd
yavs run --gui --work=lib:out/lib tb --stop-time=1ms
```

`yavs run --gui` looks for `yavs-gui` next to the `yavs` executable, then on `PATH`; when it
finds neither it stops with error `E0122` (observed). You can also start the window yourself:

```
yavs-gui [--yavs PATH] -- <yavs run arguments>
yavs-gui --attach HOST:PORT
```

- After `--` come the arguments of `yavs run` (without `run` itself); the GUI adds
  `--dap=stdio`. `--yavs PATH` names the simulator binary; without it the one next to
  `yavs-gui`, then the one on `PATH`, is used.
- `--attach` connects to a `yavs run --dap=PORT` that you started by hand, for example on another
  machine or from a script. That run waits for the client before it simulates.
- `--help` and `--version` print and exit without opening a window.

The window needs a display and an OpenGL driver at run time (software rendering works). On
Linux the X11, Wayland and EGL/GLX libraries are loaded at run time, so no development packages
are needed to build it. Closing the window ends the simulation.

## What works today

Run control, the source view and the data panels are in; the hierarchy, objects, processes and
waves panels are placeholders that name the slice that will fill them
(`docs/design/gui.md`, section 10).

| Area | State |
|---|---|
| Toolbar | Run, Pause, Stop, step Over, In and Out, Delta, Timestep, and Run for a time (a VHDL time such as `100 ns`) |
| Status bar | the stop reason with the simulation time and delta, and the process that stopped |
| Console | the run's reports and output, and the simulator's standard error |
| Source | tabs per file (a file is opened by path), a gutter with line breakpoints (click to set or clear, right click for a condition and a hit condition (the dialog calls it Hit count; the text is sent as is: `3`, `>= 3`, `% 3`), or to disable), the current line, and hover to see a value |
| Call stack, Locals, Watch | frames of the stopped process, variables with records by field and large arrays in pages, values you can edit, and watch expressions evaluated at every stop |
| Breakpoints | the list with enable, condition, hit condition, edit and remove |
| Hierarchy, Objects, Processes, Waves | not implemented yet |
| Not yet | signal breakpoints, force, deposit and release, step to the next statement of any process, exception breakpoints, keyboard shortcuts, restart, saved layout |

Stack, variables and values are served while the simulation is stopped; while it runs the
panels are greyed and ask again at the next stop. Under the debugger the design runs on one
thread, so `--threads` is ignored (warning `W0109`), and `--cov` cannot be combined with the
debugger.

## Building

`gui/` is its own Cargo workspace, outside the simulator's, so the simulator's checks never
build it. It needs Rust 1.95 or newer and fetches its dependencies, one of which is a pinned git
revision of the waveform viewer library it will embed.

```
cd gui
cargo build --release --locked
cargo test --locked
YAVS_BIN=../target/debug/yavs cargo test --locked -- --ignored   # against a built yavs
```

The tests are headless: they drive the window against a scripted debug server and need no
display. Developer notes are in `AGENTS.md`; the design is `docs/design/gui.md`.
