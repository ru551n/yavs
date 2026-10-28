# M10 GUI: yavs-gui

Status: accepted (#384, epic #148, #133); the user's decisions are in section 12. Binding inputs:
`docs/plan.md` (the `yavs-gui` row of the architecture table, "Designed for the GUI from the
start", M10) and the M8 DAP server as it is on main (`crates/yavs-dap/AGENTS.md`).

Implementation status, as of 2026-10-28: slices G1 (#388) and G2 (#399) are in
`gui/` (see `gui/AGENTS.md`), `yavs run --gui` starts it, and release archives ship it. The
Hierarchy, Objects and Processes panels (G3) and the Waves panel (G4) are still placeholder tabs.
Of the server additions in section 4, only P6 (live waves, #325) is implemented; P1 to P5 and
P7 to P9 are not, so the status bar parses time and delta from the `description` of `stopped`
and there is no global step from inside a process. The section 10 table has a status column.
The M8 server of section 1 is implemented and used by `editors/vscode`. Sections 2 to 6 describe
the design, not what runs today, unless the status column says so.

The plan's requirement, restated as the acceptance list for M10:

- debugging in the style of commercial simulator debuggers: line breakpoints with conditions and
  hit conditions; signal breakpoints on a change or on a value; step into, over and out within a
  process plus a global step to the next statement of any process; run for a time, to the next
  delta or to the next time step; call stack and locals per process; a list of processes with
  what each waits on; time and delta at every stop;
- the look of those debuggers in their modern form: dockable panels, a hierarchy tree, a source
  view with a breakpoint gutter, embedded waves;
- an egui/eframe application embedding libsurfer, in its own process, talking to `yavs run` over
  DAP. Nothing in it needs kernel changes.

## 1. What M8 already gives the GUI

The server (`crates/yavs-dap`, `yavs run --dap=PORT|stdio`) on main:

| Area | Requests and events | Notes |
|---|---|---|
| Session | `initialize`, `launch`/`attach`, `configurationDone`, `disconnect`/`terminate`; events `initialized`, `output`, `exited`, `terminated` | `launch` reads only `stopOnEntry`; the design comes from the command line |
| Processes | `threads` (thread = process, id = index + 1, named after the process) | names only, no state |
| Run control | `continue`, `next`, `stepIn`, `stepOut`, `pause`; `yavs/runFor`, `yavs/runUntil` (`time`: a VHDL time literal), `yavs/stepDelta`, `yavs/stepTimestep`; events `stopped`, `continued` | at a stop between cycles every step is a global step |
| Line breakpoints | `setBreakpoints` with `condition` (VHDL expression, evaluated by the server) and `hitCondition`; response gives the line it slid to | `supportsConditionalBreakpoints`, `supportsHitConditionalBreakpoints` |
| Signal breakpoints | `dataBreakpointInfo`, `setDataBreakpoints` (on any change, with condition and hit condition); `yavs/setSignalBreakpoints` (on a value) | each request replaces its own set |
| Inspection | `stackTrace`, `scopes` (Locals, Signals, Generics, Hierarchy), `variables` (records by field, arrays paged by index), `evaluate` (locals and signals by name or path, also for hovers) | served only while stopped; Locals only for the process that is executing |
| Changing values | `setVariable` (local: written at once; signal: deposit), `yavs/force`, `yavs/deposit`, `yavs/release` (whole signal, VHDL literal) | applied in the next cycle |
| Hierarchy | `yavs/hierarchy` (any time): regions with name, path, kind, child regions, process names, signals with path and type | static |
| Time and delta | the `description` of `stopped`: `"<reason> at <time>, delta <n>"` | text only |

Also on main: the editor extension in `editors/vscode/` (a client of the same requests, useful as
a second reference client; its user guide is `editors/vscode/README.md`), and the wave recorder `crates/yavs-wave` (`yavs run --wave`: FST
through `wavefst`, hierarchy with records and arrays as scopes, enum tables, include and exclude
globs, an array size cap, final value per time step). Live waves and `--wave`/`--vhpi`
together with `--dap` (#325) are in (section 5.1).

## 2. Process model

```
 yavs-gui (EUPL-1.2, gui/)                          yavs (MIT OR Apache-2.0)
+--------------------------------+   stdio pipes   +------------------------------+
| UI thread: eframe, panels,     |  DAP (framed    | yavs run ... --dap=stdio TOP |
|   libsurfer SystemState        |  JSON) -------> |   reader thread: requests    |
| DAP reader thread: child stdout|  <------------- |   simulation thread: stops,  |
|   -> channel, request_repaint  |  events, waves  |   run loop, wave stream      |
| child stderr -> console        |                 |                              |
+--------------------------------+                 +------------------------------+
```

- **Launch** (default): `yavs-gui [--yavs PATH] -- <yavs run arguments>`. The GUI spawns
  `yavs run <arguments> --dap=stdio` as a child (the arguments are those of `yavs run`, without
  the subcommand) with piped stdin, stdout and stderr. The yavs command
  line stays the one contract for the design, as for the editor extension
  (`editors/vscode/AGENTS.md` section 3). The binary is `--yavs`, else `yavs` next to the
  `yavs-gui` executable, else `yavs` on `PATH`; none found is an error naming the three places
  looked at, never a guess.
- **Attach**: `yavs-gui --attach HOST:PORT` connects to a `yavs run --dap=PORT` started by the
  user (remote machines, a test runner that starts yavs itself). Same client code over a
  `TcpStream`.
- **Restart** is a new child: the GUI disconnects, kills the old child if it does not exit within
  2 s, starts the command again and replays its client state (breakpoints, signal breakpoints,
  wave subscription) before `configurationDone`. The dock layout and the wave view (libsurfer
  `LoadOptions::KeepAll`) stay. No in-process restart request is needed (elaboration is
  re-entrant, plan rule 4, so one can be added later if relaunching proves slow).
- **Re-analysis** before a restart: an optional `--analyze "<yavs analyze arguments>"` that the
  GUI runs first, its diagnostics shown in the console. The GUI never calls yavs crates.
- **Threads in the GUI**: the UI thread owns all state; one reader thread parses framed messages
  (`Content-Length`, as `crates/yavs-dap/src/wire.rs`) into `serde_json::Value`, sends them over
  an `mpsc` channel and calls `egui::Context::request_repaint`; writes go through a mutex on the
  child's stdin. Each request gets a `seq`; the UI keeps `seq -> purpose` and handles the
  response when it arrives. No async runtime of our own.
- **Running versus stopped**: the server answers inspection requests only while stopped (`the
  simulation is running` otherwise). While running the GUI keeps the last stop's stack and
  locals greyed, shows live waves and the latest time (section 5), and enables only Pause and
  Stop.
- Under `--dap` the simulation runs one thread (`W0109` for more): sessions are deterministic.

## 3. Panels and the requests they use

| Panel | Uses (on main) | Needs (section 4) |
|---|---|---|
| Toolbar | `continue`, `pause`, `next`, `stepIn`, `stepOut`, `yavs/runFor`, `yavs/runUntil`, `yavs/stepDelta`, `yavs/stepTimestep`, `disconnect` | `yavs/stepAny` (global step from a process stop) |
| Status bar | `stopped`, `continued`, `exited` | P1 structured time and delta; `yavs/time` while running (in P6) |
| Hierarchy tree | `yavs/hierarchy` | P3 source locations of regions, processes and signals |
| Objects (signals of the selected region, with values) | `evaluate` per signal, or the Hierarchy scope | P4 `yavs/values` (many paths in one request) |
| Processes | `threads` | P2 `yavs/processes` (state, wait statement, sensitivity, timeout) |
| Source view and gutter | `setBreakpoints` (condition, hit condition), `stackTrace` (current line), `evaluate` (hover); file text read from disk | P5 `breakpointLocations` (which lines can hold a breakpoint), `loadedSources` (file list) |
| Call stack | `stackTrace` per process | Locals of suspended processes (P7) |
| Locals and Watch | `scopes`, `variables` (paged), `evaluate`, `setVariable` | P7 |
| Breakpoints | `setBreakpoints`, `dataBreakpointInfo`, `setDataBreakpoints`, `yavs/setSignalBreakpoints`; the list itself is client state | P8 exception filters (stop on assertion severity, on a runtime error) |
| Force and deposit dialog | `yavs/force`, `yavs/deposit`, `yavs/release` | none |
| Waves (libsurfer) | none yet | P6 live wave stream (#325) |
| Console | `output` events, child stderr | none (commands typed there map to the requests above, parsed by the GUI) |

## 4. Missing protocol pieces

All of them are additions to `crates/yavs-dap` (plus a sink seam in `crates/yavs-wave` for P6),
in the MIT/Apache workspace, each with scripted-session tests in `crates/yavs/tests/dap/`. None
touches the kernel: the data is in `DebugSession::processes` (`ProcStatus`: state, wait statement,
sensitivity, timeout), `Design` (statement locations, hierarchy), the observer accessors the wave
recorder already uses, and the value-change bus.

| # | Addition | Shape | Source of the data |
|---|---|---|---|
| P1 | Structured time and delta at every stop | `stopped` body gains `"yavs": {"time": <fs, integer>, "timeText": "10 ns", "delta": <n>}`; the `description` stays | `stop.now`/`stop.delta`, `sim.now()`/`sim.delta()` |
| P2 | Process list with wait reasons | `yavs/processes` (any time; consistent at a stop) -> `{processes: [{threadId, name, path, state: "notStarted"\|"running"\|"waiting"\|"terminated", source: {path, line} of the process, wait: {path, line} or null, sensitivity: [signal path], timeout: <fs> or null}]}` | `ProcStatus`, `Design` statement locations |
| P3 | Source locations in the hierarchy | `yavs/hierarchy` regions, processes and signals gain `source: {path, line}` (declaration or instantiation) | `Design` (spans of the elaborated objects) |
| P4 | Many values in one request | `yavs/values {paths: [...]}` at a stop -> `{values: [{path, value, type, variablesReference}]}`; a bad path is a per-entry `error`, not a failed request | `Design::Sig` decode, the per-stop node table |
| P5 | Source navigation | standard `breakpointLocations` (lines with statements in a file, `Design::line_stmts`) and `loadedSources` (the design's files); capabilities `supportsBreakpointLocationsRequest`, `supportsLoadedSourcesRequest` | `Design` line table |
| P6 | Live waves (#325) | see section 5: `yavs/waveSubscribe`, `yavs/waveUnsubscribe`, event `yavs/waves`; plus `--wave` working under `--dap` | `yavs-wave` hierarchy and value bytes, value-change bus |
| P7 | Locals of suspended processes | `scopes` gives Locals for every waiting process's frame 0 (process variables), read through the same accessor `--wave-variables` uses; read-only unless the process is the one stopped | `Hook::with_process`, `InterpProcess::var` |
| P8 | Exception filters | `exceptionBreakpointFilters`: `assert-error`, `assert-failure`, `runtime-error` (and `assert-warning`, `assert-note`, off by default); `setExceptionBreakpoints` arms them; the stop has reason `exception` at the reporting statement, before the run ends | needs a hook where the interpreter raises a report or a runtime error: check `yavs-interp` first; if it is not there, an interpreter hook is the change, not the kernel |
| P9 | Global step from inside a process | `yavs/stepAny` -> `Resume::GlobalStep` (exists in `yavs-interp`, only reachable between cycles today) | `DebugSession` |

Not needed: a custom restart (section 2), a custom source request (files are read from disk, paths
are absolute), an objects-while-running request (the Objects panel reads the wave store then).

## 5. Live waves into libsurfer

### 5.1 On the wire (P6, closes #325)

- `yavs/waveSubscribe {include: [glob], exclude: [glob], maxArrayElems, variables: bool,
  flushMs}` before `configurationDone` (or later, with history only from then on; it replaces
  the subscription). Globs and cap are those of
  `yavs_wave::Selection`. The response is the wave hierarchy as `yavs-wave` builds it for FST:
  scopes (`kind`, name, generate alternative), and per variable an `id`, its `SigType` (vector
  width and element enumeration, enum literal table in original spelling, integer subtype name and
  width, real, time) and direction. The GUI then builds the same tree a written FST would show.
- Event `yavs/waves {from, to, times: [<fs>], changes: [[<time index>, <id>, <value>]]}`: the
  changes of completed time steps, final value per time step as in the FST, in entry order. The
  value is base64 of the value bytes the recorder gives the FST path (user, 2026-10-19: "binary
  now"): a bit/`std_logic` vector one byte per element (the enumeration position, left to
  right), an enumeration a `u32`, an integer or TIME (fs) an `i64`, a real an `f64`, little
  endian. The subscribe response gives each signal an `encoding {layout: "u8-per-element" |
  "u32le" | "i64le" | "f64le", width}` beside its `type`, so the GUI converts bytes straight to
  its store. One string per change; a blob per batch with offsets would not be smaller for
  values of a few bytes. A batch with no changes still carries `to`, so the status bar time
  moves while running.
- The server flushes a batch at the end of a time step once `flushMs` (default 100) have passed
  or 256 KiB are pending, always before `stopped`, `exited` and the response to
  `yavs/waveUnsubscribe`, so the GUI's waves end where the stop is.
- Server side, the `Recorder` has a sink seam (`yavs_wave::Sink`, `record_to`): the FST
  `WaveWriter` and the DAP stream are two sinks, so `--wave out.fst` and the stream work together
  under `--dap`.
- The simulation never waits for the client (user decision, 2026-10-19): batches go through a bounded
  queue to a sender thread; when it is full the newest batch is merged into the last one, keeping
  only the last change of each value (`merged: true`). `ponytail:` base64 in JSON on the DAP pipe;
  a binary side channel is the upgrade if measured throughput needs it.

### 5.2 In the GUI

libsurfer is not on crates.io; its public API (checked on its main branch, version 0.7.0, egui
0.36, wellen 0.25.6) offers what the GUI needs without a fork:

- `SystemState::new()` and `eframe::App::ui(&mut self, ui, frame)`: libsurfer draws into the `Ui`
  it is given, with `Panel::left(...).show(ui, ...)` inside it, so a dock tab can host it.
  `SystemState::update(Message)` drives it.
- `Message::WavesLoaded(WaveSource::Data, WaveFormat::Fst, Box<WaveContainer>, LoadOptions)` with
  `WaveContainer::Wellen(Box<WellenContainer>)`, `WellenContainer::new(Arc<Hierarchy>, None,
  None)` and `WellenContainer::add_body(BodyResult::Local(wellen::viewers::BodyResult {source,
  time_table}))` loads a waveform built in memory. `LoadOptions::KeepAll` keeps the displayed
  signals, cursor and zoom across reloads.
- `Message::AddVariables`, `Message::AddScope`, `Message::GoToTime`, `Message::CursorSet`,
  `Message::SetMenuVisible(false)`, `Message::SetSidePanelVisible(..)`, `Message::SetToolbarVisible(..)`
  let the GUI add signals from its own hierarchy and objects panels, move the cursor to the stop
  time and hide libsurfer's own menu so the GUI's toolbar is the only one.

The live store: the GUI builds a `wellen::Hierarchy` with `wellen::HierarchyBuilder` from the
subscribe response (same mapping as the FST: 9-state characters for the std_logic family, enum
tables, integer widths) and feeds every batch into a `wellen::Encoder` (`time_change`,
`vcd_value_change` with the value converted by its type). On every stop, and at most twice a second while
running, it calls `Encoder::snapshot()` and hands libsurfer a new container with
`WavesLoaded(.., KeepAll)`.

`ponytail:` `snapshot()` clones the encoder, so each refresh costs the size of the recorded data.
Upgrade path: a small upstream wellen change that makes `Signal::new_fixed_len` usable from outside
(its `FixedWidthEncoding` argument is not exported today), then a `SignalSourceImplementation` (a
public trait) over the GUI's append-only store, so a refresh copies only the signals on screen.

Rejected: reading a growing FST written by `--wave`. The writer flushes every 5 s and patches the
header, libsurfer re-reads the whole file on reload, and its file watcher is not built on Windows.
Rejected: libsurfer's other live source, whose protocol is bit-vector based and would lose enum
tables and records.

Waves show completed time steps. The values of the current time step at a stop inside a cycle are
in the Objects and Locals panels; delta cycles are not drawn as their own time axis.

After the run the GUI keeps the store (the waves stay browsable); `File > Save waves` is
`--wave` given to the next run, not a second writer in the GUI.

## 6. Layout

A modern dark and light theme (egui `Visuals`, follows the system setting), one window, dockable
panels with tabs (`egui_dock` `DockArea`, layout saved with `serde` in the eframe storage and
reset from the View menu). Default arrangement:

```
+--------------------------------------------------------------------------------+
| Menu | Run Pause Stop | Step Over In Out | Step Any | Delta | Timestep |
|      | Run for [ 100 ns ] | Run until [ ... ]          time 1250 ns  delta 3     |
+------------------+-------------------------------------------+-----------------+
| Hierarchy        | Source (tabs per file)                    | Call stack      |
|  tree of regions |  gutter: breakpoints (red, hollow when    |  per process    |
|  with processes  |  disabled, + condition marker), current   +-----------------+
|  and signals     |  line arrow, executable-line ticks        | Locals | Watch  |
+------------------+                                           |  tree, paged    |
| Objects          |                                           |  arrays, edit   |
|  signals of the  +-------------------------------------------+-----------------+
|  selected region | Waves (libsurfer)                         | Processes       |
|  name value type |  cursor follows the stop time             |  state, waits on|
+------------------+-------------------------------------------+-----------------+
| Console | Breakpoints                                                          |
+--------------------------------------------------------------------------------+
| status: stopped (breakpoint) at 1250 ns, delta 3  process /tb/dut/p_main     |
+--------------------------------------------------------------------------------+
```

Panel behaviour:

- **Hierarchy**: lazy tree; selecting a region fills Objects; context menu: add to waves
  (`AddScope`), open source (P3), break on change (signal).
- **Objects**: name, value, type for the region's signals; values from P4 at a stop and from the
  wave store while running; context menu: add to waves, break on change, break on value, force,
  deposit, release.
- **Source**: read-only text view with VHDL keyword colouring (a small lexer of its own over the
  file text, no parser dependency); click in the gutter toggles a breakpoint, right click edits
  condition and hit condition; hover evaluates; the current statement is highlighted per selected
  process.
- **Call stack**: the stopped process first, then any process chosen in Processes (a waiting
  process shows the `wait` it is on, as `stackTrace` does today).
- **Locals and Watch**: the DAP variables tree; arrays paged with `start`/`count`; double click
  edits (`setVariable`); watch expressions re-evaluated at each stop.
- **Processes**: one row per process: path, state, waits on (sensitivity list or the `wait`
  statement's location and timeout); selecting a row selects that thread in Call stack and Source.
- **Breakpoints**: line, signal and exception breakpoints with enable, condition, hit condition and
  hits so far; the GUI owns the list and resends a whole set per request kind.
- **Console**: `output` events (reports, assertions, runtime errors in red) and a command line for
  `run 100 ns`, `step`, `force /tb/x '1'` and the like, each one request from section 1 or 4.

Keyboard: F5 run/continue, Shift+F5 stop, F6 pause, F10 step over, F11 step in, Shift+F11 step
out, F9 toggle breakpoint, configurable through the eframe storage.

## 7. Crate and licensing boundary

- `gui/` at the repository root is its own Cargo workspace (like `fuzz/`), with one crate
  `yavs-gui` (binary), `license = "EUPL-1.2"` and a `LICENSE` file with the EUPL-1.2 text. It is
  EUPL-1.2 because libsurfer is.
- `yavs-gui` depends on no yavs crate. It speaks DAP over a pipe or a socket, so the MIT/Apache
  crates never link EUPL code and the licence boundary is a process boundary. The protocol is
  documented in section 1 and 4 and pinned by the server's transcript tests.
- Dependencies: `libsurfer` (git, pinned `rev`, `default-features = false`, which turns off its
  HTTPS and performance-plot features), `eframe`/`egui` at the version libsurfer pins, `egui_dock`
  (MIT), `wellen` at the version libsurfer pins (its `Hierarchy` crosses the API), `serde_json`.
  Everything is fetched and built by cargo (root `AGENTS.md` rule 9); libsurfer's build script runs
  `git` to stamp its version, which a git dependency checkout satisfies.
- The root workspace (`members = ["crates/*"]`) does not include `gui/`, so the rule 13 checks of
  every agent do not build libsurfer. `gui/` gets its own checks (section 9).
- Release archives: the same archive as `yavs`, with `yavs-gui` next to `yavs` and its licence
  as `LICENSE-EUPL-1.2-yavs-gui`, which applies to the `yavs-gui` binary only (section 12).
- `yavs run --gui <arguments>` starts `yavs-gui --yavs <this yavs> -- <arguments>` (a process
  start, no linking): `yavs-gui` next to the `yavs` executable, else on PATH, else `E0122`.

## 8. Platforms

- Linux x86_64 and aarch64: eframe with the `glow` renderer and both `x11` and `wayland`
  (libsurfer's own choice). Expected: no development packages at build time (the windowing
  libraries are loaded at run time) and an OpenGL driver at run time, software rendering on
  machines without a GPU. G1 verifies both on the CI images.
- Windows x86_64: the same renderer; file paths in DAP are absolute and compared case-insensitively
  as the server already does (`crates/yavs-dap/AGENTS.md`).
- macOS stays out of scope (plan), but nothing in `gui/` may be Unix-only.
- Minimum Rust: what libsurfer declares (1.95 on its main branch), below the toolchain yavs uses.

## 9. Testing

- **Protocol** (MIT side): every P-addition gets scripted sessions in `crates/yavs/tests/dap/`
  (`.script` in, `.expected` transcript), as for the M8 requests. This pins what the GUI relies on.
- **GUI logic without a window**: the client state (breakpoint sets, stop handling, process and
  variables trees, wave store) lives in plain structs updated from DAP messages and tested with
  `cargo test` alone.
- **Headless egui**: `egui_kittest` (at libsurfer's egui version) drives the panels through
  their accessibility tree: click the gutter, expect a `setBreakpoints` with that line; press F10,
  expect `next`; feed a `stopped` event, expect the status text and the highlighted line. No
  image snapshots (they need a GPU backend on every CI platform); layout is reviewed by hand.
- **DAP mock**: a test server in `gui/tests/` that answers from a scripted list of
  `(expected request, response, events)` over an in-memory pipe, the same transcript idea as the
  server's tests. It checks the GUI sends what it should and handles what it gets, including the
  `the simulation is running` failure and an early child exit.
- **End to end**: one test starts the real `yavs` (`YAVS_BIN`, built from the root workspace in the
  same CI job) on the `editors/vscode/sample` testbench: breakpoint hit, step, a signal value
  breakpoint, force, live waves present at the stop time, run to the end. Under 1 minute
  (root rule 16).
- **CI**: a `gui` job on the three platforms (`cargo fmt`, `clippy -D warnings`, `test` in
  `gui/`), separate from the rule 13 job so a libsurfer update cannot block the simulator.

## 10. Phased plan

Each slice is one agent, one issue under epic #148. Server slices come first and are useful to
the editor extension too; the GUI slices after G1 can run in parallel with them once their P-items
are on main.

| Slice | Where | Content | Done when | Status (2026-10-28) |
|---|---|---|---|---|
| S1 | `yavs-dap` | P1 structured time and delta, P2 `yavs/processes`, P9 `yavs/stepAny` | transcripts for each; editor extension shows time and delta from P1 | not started (#396) |
| S2 | `yavs-dap` | P3 source locations in `yavs/hierarchy`, P4 `yavs/values`, P5 `breakpointLocations`, `loadedSources` | transcripts | not started (#397) |
| S3 | `yavs-dap`, `yavs-wave` | P6 live waves (#325): subscribe, batches, `Recorder` sink seam, `--wave` and `--vhpi` under `--dap` | transcripts; an FST written under `--dap` reads back with wellen and equals the streamed changes | P6 live waves done (#325); `--wave` and `--vhpi` under `--dap` done |
| S4 | `yavs-dap` (maybe `yavs-interp`) | P7 locals of suspended processes, P8 exception filters | transcripts; a failing assertion stops at its statement | not started (#398) |
| G1 | `gui/` | workspace, licence, eframe app, `egui_dock` layout with empty tabs, DAP client (launch and attach), toolbar run controls, status bar, console, mock server, kittest harness, CI job | the GUI launches the sample testbench, runs it to the end and shows its reports; tests green on three platforms | done |
| G2 | `gui/` | source view with gutter, line breakpoints with condition and hit condition, call stack, Locals and Watch, hover, Breakpoints panel | breakpoint hit, step in/over/out and global step on the sample; mock and kittest tests | done (hit counts so far are not shown; no global step, S1) |
| G3 | `gui/` | Hierarchy, Objects, Processes panels; signal breakpoints (change and value); force, deposit and release | the acceptance list items for these panels, with tests | not started |
| G4 | `gui/` | libsurfer embedding: wellen hierarchy from the subscription, encoder store, snapshot refresh, add to waves from Hierarchy and Objects, cursor at the stop time | live waves on the sample, kept across a restart; store test against an FST of the same run | not started |
| G5 | `gui/` | restart and `--analyze`, layout persistence and reset, shortcuts, themes, console command line | manual checklist in `gui/AGENTS.md`, kittest for the commands | not started |
| G6 | `gui/`, release | end-to-end test against the real `yavs`, release archives, user documentation page | M10 check: the acceptance list in the introduction, walked through on the sample on Linux and Windows | not started (release archives already ship `yavs-gui`) |

## 11. Ceilings and what is left out

- Waves end at the last completed time step; no delta-cycle axis.
- The wave refresh clones the encoder (section 5.2); fine for testbench-sized runs, the upgrade is
  named there.
- A slow GUI slows the simulation (section 5.1); nothing is dropped.
- No drivers view, schematic or dataflow view, memory viewer of our own, or coverage view in M10;
  each can be one more panel over one more request later.
- No editing of sources in the GUI.

## 12. Decisions (user, 2026-10-19)

Answered on #384; recorded in `docs/decisions.md` "GUI (M10)".

1. **libsurfer as a git dependency** pinned to a revision of its main branch (`gui/Cargo.toml`),
   not a local checkout (#133); it is not published on crates.io. `default-features = false`
   turns off its HTTPS and performance-plot features.
2. **`gui/` is its own workspace** outside `crates/*`, EUPL-1.2, with its own CI workflow
   (`.github/workflows/gui.yml`); it depends on no yavs crate and talks DAP only, so the
   simulator's checks never build the GUI.
3. **Live waves stream over DAP** into an in-memory wellen store (section 5), not a growing FST.
4. **`egui_dock`** for docking, as recommended in section 6 (the user left the choice open; the
   recommendation stands).
5. **Shipping**: `yavs-gui` is in the same release archive as `yavs`, with its EUPL-1.2 licence
   file alongside (`LICENSE-EUPL-1.2-yavs-gui`), marked as applying to the `yavs-gui` binary
   only.
6. **`yavs run --gui` exists** as a shortcut that starts `yavs-gui` on the run: `yavs-gui` next to
   the `yavs` binary or on PATH, else error `E0122` (exit 2). The GUI is always a separate
   process.
