# AGENTS.md: yavs-dap

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-dap`.
The design is `docs/plan.md` (`yavs-dap`, M8) on the interpreter's debug hooks,
`docs/design/ir.md` 3.4.

## 1. Purpose and boundaries

- The Debug Adapter Protocol server behind `yavs run --dap=PORT|stdio`: builds the simulation
  with every process attached to one `yavs_interp::DebugSession`, runs it, and answers a DAP
  client: breakpoints, stepping, pause, stack, variables, evaluate, output, exit.
- Does not: elaborate or lower (the CLI hands it an `Elaborated` with IR), parse the command line,
  or drive the kernel other than through `SimControl` and the session (plan: nothing else drives
  simulation).
- Depends on: `yavs-diag`, `yavs-elab`, `yavs-interp`, `yavs-ir`, `yavs-kernel`, `yavs-sem`,
  `yavs-logic`, `yavs-std` (TIME literals, `value::literal` for values typed by the user),
  `yavs-syntax` (breakpoint conditions), `yavs-wave` (live waves), `serde_json`. Called by: `yavs` (`src/elab.rs`, `run`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `serve` (transport, session, run loop, end of run), `Transport`/`parse_transport`, `Shared` state, reader-thread requests (`handle`, `set_breakpoints`, `parse_hit`, `data_breakpoint_info`, `set_signal_breakpoints`), the run loop's `Run`s and `signal_hit`, stop-time requests (`Front::on_stop`, `serve_stop`, `stack_trace`, `scopes`, `variables`, `evaluate`, `signal_op`, `set_variable`, `write_node`), the per-stop variables table (`Node`, `node_ref`, `value_var`, `region_vars`), names for conditions and evaluate (`Names`), line conditions (`LineCond`, `fires`), `hierarchy`, the force/deposit/release queue (`Op`, `queue`, `apply_ops`), `Output` observer |
| `src/cond.rs` | Breakpoint conditions: `parse` (a VHDL expression through `yavs-syntax`, unsupported constructs rejected), `Cond::eval` over an `Env` of `Lit` values (predefined operators; literals read in the other operand's type) |
| `src/design.rs` | `Design`: what the debugger shows, extracted from `Db`/`Elaborated` before the run: statement locations, line table, the hierarchy (`Region`, `roots`), signals (`Sig`: `Fmt` walks its kernel leaves with its `LitType`, `value`/`show` decode, `parse`/`encode` for deposits and value breakpoints), locals (`ir_lit`, `to_const` between interpreter values and `Lit`), per process its regions, signals and generics |
| `src/waves.rs` | Live waves (#325): `Tap` (the bus observer forwarding to the subscription's `yavs_wave::Recorder`, started late at the time of the values), `StreamSink` (the recorder's `Sink`: values as base64 of the recorder's bytes, batches per `flushMs`), `Stream` (pending batch, bounded queue, merge when full), `sender` thread, `hierarchy` (subscribe response, with each signal's `encoding`) |
| `src/wire.rs` | `Content-Length` framing (`read_message`, `write_message`) |
| `../../editors/vscode/` | The VS Code extension and its client and in-editor tests (run them after changing this crate, `editors/vscode/AGENTS.md`) |
| `../yavs/tests/dap.rs`, `../yavs/tests/dap/` | Scripted sessions over stdio (`NAME.script` in, `NAME.expected` transcript, `YAVS_BLESS=1`), pause, TCP |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Processes run on one thread under `--dap` (`Config::threads = 1`, the CLI warns `W0109` for more) | A stop inside a process is mid-delta; one thread makes every stop a defined point and sessions identical at any `--threads` | ir.md 3.4, `docs/decisions.md` "DAP server" |
| DAP threads are processes: `threadId` = session process index + 1, named by `Elaborated::proc_name` | One debuggable sequential thing per process | plan `yavs-gui` debugging list |
| Frame id `threadId << 16 \| depth` (outermost 0), variables reference `frameId << 2 \| scope` (0 hierarchy, 1 locals, 2 signals, 3 generics); references from `CHILD` (2^32) on index a table cleared at every stop (`Shared::nodes`): a composite's children (local or signal plus a field and element path, re-read on each request) or a hierarchy region | Scopes stay stateless (data breakpoint info decodes them on the reader thread); children need a path | this file |
| Composites expand: records by field (`namedVariables`), arrays by index with `indexedVariables`, so a client pages them (`start`, `count`); children are named by field or index value | VS Code chunks large arrays itself | issue #326 |
| The Hierarchy scope (expensive) and `yavs/hierarchy` (reader thread, static: regions, process names, signals) show the elaborated regions without processes and component scopes; in the scope a process shows its state | One tree for the Variables view, one for clients that draw their own | issue #326 |
| Requests that need no stop (`initialize`, `launch`, `setBreakpoints`, `configurationDone`, `threads`, `pause`) are answered on the reader thread; the rest go to the simulation thread and are served only while stopped (`the simulation is running` otherwise) | The simulation thread is busy inside `SimControl::run_until` | this file |
| A line breakpoint is set on the outermost statements starting on the line, or the next line with any (`Design::line_stmts`); the response gives the actual line | Editors expect breakpoints to slide to code | this file |
| Pause sets both the session pause (a running process stops at its next statement) and the kernel `Interrupt` (between cycles); the kernel pause that follows a process stop is swallowed (`paused_in_process`), an untaken session pause cancelled (`DebugSession::cancel_pause`) | An endless loop inside a process must be pausable; one pause is one stop The `pause` response is sent before anything is paused, so the stop never reaches a client ahead of it (#441) | this file |
| `yavs/runFor`/`yavs/runUntil`/`yavs/stepDelta`/`yavs/stepTimestep` are `SimControl::run_until`/`step_delta`/`step_timestep` in the run loop (`Run`); from a stop inside a process they are queued (`Shared::queued`) and the kernel paused, so they start once the cycle has ended (a delta step is then that end) | Nothing but `SimControl` drives the kernel; a process stop is mid-cycle | issue #324 |
| A stop during a run to a time does not cancel it: the run loop keeps the limit until it is reached, so `continue` resumes it | One rule for line, signal and pause stops; matches a `run` command resumed after a breakpoint | this file |
| Custom run requests answer, then send `continued` (thread of the stop, or 1) | A client marks the session running only for the standard resume requests | VS Code |
| Signal breakpoints are one `SignalBreak` observer, subscribed before the run, whose shared watch list (`Watches`) holds every leaf of every signal breakpoint, on any change; `setDataBreakpoints` and `yavs/setSignalBreakpoints` each replace their own set (`Shared::sig_bps`), answered on the reader thread | The interpreter's signal breakpoint and the event bus, no second watcher; VS Code resends all data breakpoints on every change and must not drop the custom ones |
| Values the user types (value breakpoints, `yavs/force`, `yavs/deposit`, `setVariable`) are parsed for the target's type by `yavs_std::value::literal` (scalars as `'VALUE` reads them, any unit of a physical type, string and bit string literals, aggregates) and compared or written as `Lit`; after a pause with hits, a change that matches no value breakpoint resumes the run | One value syntax, VHDL's, for every request; the Variables view writes values in it | issues #322, #323 |
| Force, deposit and release act on a signal or a part of it (`split_part`: the longest prefix that names a signal; `select`: fields, integer indices, a last slice in the signal's direction) through `SimControl::force`/`deposit`/`release`, one call per kernel element of the part (`Sig::part`, `Fmt::part`; issue #364), so only that part changes; a deposit encodes the current value with the part replaced; a request at a stop only queues them (`Shared::ops`), the run loop makes them before it runs on, so they take effect in the next cycle. From a stop inside a process the kernel is paused after the current cycle, as for a queued run, and that pause is no stop | Nothing but `SimControl` drives the kernel, and a process stop is mid-cycle; one path for both kinds of stops | issue #323 |
| `setVariable` on a signal is a deposit (of only the field or element set); on a local it writes the variable at once (`FrameView::set_value`, the whole value, composites keep their bounds); generics are refused | The Variables view's "Set Value" does what a user expects of each | issue #323 |
| Signal variables carry `__vscodeVariableMenuContext: "signal"` | VS Code's context key for the extension's force, deposit and release menu entries | VS Code |
| Breakpoint conditions are evaluated by the server (`cond.rs`) over the values it shows, not lowered to IR: names are the stopped frame's and process frame's locals, then signals seen from the process (for a data breakpoint set without a frame, from the signal's region) or by path; predefined operators only. A condition that does not parse or uses something unsupported is `verified: false` with the message; an error while evaluating is a stop with reason `exception`. A line breakpoint with a condition is an unconditional interpreter breakpoint; a stop whose condition does not hold returns `Resume::Skip`, so a step under way goes on. Hit conditions count the hits where the condition held (line and data breakpoints) | A running program cannot take new IR functions; most conditions compare locals and signals with literals | issue #326 |
| Live waves (`yavs/waveSubscribe`/`yavs/waveUnsubscribe`, reader thread, any time) are a `yavs_wave::Recorder` built by `record_to` with the client's `Selection` into `waves::StreamSink`; the bus observer `Tap` forwards to it, and one subscribed after the start records every value from its first callback on. Batches (`yavs/waves`) hold completed time steps, values base64 of the recorder's value bytes, decoded by the hierarchy's `encoding` (user, 2026-10-19), sent at a step end after `flushMs` or 256 KiB and always before `stopped` (`serve_stop`), `exited` (`Recorder::finish`) and the unsubscribe response | Same hierarchy, selection and final value per time step as `--wave`; the compact encoding the GUI design proposed | issue #325, `docs/design/gui.md` 5.1 |
| Wave batches go through a bounded queue (`waves::Stream`) written by a sender thread and by `Shared::send` before every other message; a full queue merges the newest batch into the last (last change per value, `merged`) | The simulation never waits for a client; batches keep their order with the rest of the session, so transcripts are deterministic | issue #325 |
| `serve` takes `Arc<Db>`/`Arc<Elaborated>` (a subscription builds a recorder on the reader thread) and an `attach` callback: the CLI subscribes its `--wave` and `--vhpi` observers to the debug build and gets a `Finish` run before `exited`; its `Report` callback receives the final kernel statistics and interpreter runtime for `YAVS_COUNTERS` | One observer setup and counter report for `yavs run` with and without `--dap` | issues #325, #696 |
| From a stop between cycles, every step request is a global step (next statement of any process) | No process is current there | ir.md 3.4 |
| Reports and `OUTPUT` text are `output` events (stdout carries DAP in `stdio` mode); runtime errors an `output` event of category `stderr` | stdout is the protocol channel | `docs/cli-contract.md` 5 |
| Disconnect stops the run (`Interrupt::stop`), then `exited`, `terminated` and the disconnect response, in that order | Deterministic transcript | this file |
| Over TCP the server shuts down its write side after the last message and waits up to 2 s for the client to close before exiting | Exiting with the socket open resets the connection on Windows and the client loses the last messages | CI, Windows |
| Paths in frames and breakpoints are `std::path::absolute` of the source path (lower-cased as a key on Windows) | No `\\?\` prefixes; clients differ in drive letter case | this file |

## 4. Invariants and pitfalls

- `Front::on_stop` runs on the simulation thread inside a process's statement; it must not call
  `SimControl`. It reads signals through `Stop::ctx` (`At::read`).
- `Shared::rx` is locked only by the simulation thread (configuration wait, stops, end of run).
- The transcript tests depend on message order: only send from the simulation thread while it
  runs, and keep reader-thread answers to requests a client sends while stopped or before start.
- A queued force or deposit is lost when the session ends before the next cycle (disconnect).
- The wave queue is written with `out` locked first, then the queue (`Shared::send`, the sender
  thread); never the other order. A subscription's recorder is locked inside `Shared::wave`.
- Under `--dap=stdio` the CLI first dup2s descriptor 1 onto stderr (`yavs_vhpi::divert_stdout`) and hands the server a private duplicate of the original stdout (`set_stdout`), so plugin and foreign-library prints cannot corrupt the framing (#389). Any new stdout write in the CLI goes to stderr in such a run.
- Codes owned: `E0700`-`E0799`/`W0700`-`W0799` (in use: `E0701` connection cannot be opened,
  `E0702` live waves failed).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-dap -p yavs --test dap
YAVS_BLESS=1 cargo test -p yavs --test dap      # rewrite tests/dap/*.expected; review the diff
yavs run --work=lib:out/lib tb --dap=4711       # then attach an editor to 127.0.0.1:4711
```

## 6. Open issues

#389 VHPI plugin stdout under `--dap=stdio`; #364 conditions with function calls and attributes
(forcing parts is done; indices of enumeration-indexed arrays are not parsed in a part); `docs/design/gui.md` P1-P9 (the GUI's further requests).
