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
  `serde_json`. Called by: `yavs` (`src/elab.rs`, `run`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `serve` (transport, session, run loop, end of run), `Transport`/`parse_transport`, `Shared` state, reader-thread requests (`handle`, `set_breakpoints`, `parse_hit`, `data_breakpoint_info`, `set_signal_breakpoints`), the run loop's `Run`s and `signal_hit`, stop-time requests (`Front::on_stop`, `serve_stop`, `stack_trace`, `scopes`, `variables`, `evaluate`, `signal_op`, `set_variable`, `write_local`), the force/deposit/release queue (`Op`, `queue`, `apply_ops`), `Output` observer |
| `src/design.rs` | `Design`: what the debugger shows, extracted from `Db`/`Elaborated` before the run: statement locations, line table, signals (`Sig`: `Fmt` walks its kernel leaves with its `LitType`, `value`/`show` decode, `parse`/`encode` for deposits and value breakpoints), locals (`ir_lit`, `to_const` between interpreter values and `Lit`), per process its regions, signals and generics |
| `src/wire.rs` | `Content-Length` framing (`read_message`, `write_message`) |
| `../../editors/vscode/` | The VS Code extension and its client and in-editor tests (run them after changing this crate, `editors/vscode/AGENTS.md`) |
| `../yavs/tests/dap.rs`, `../yavs/tests/dap/` | Scripted sessions over stdio (`NAME.script` in, `NAME.expected` transcript, `YAVS_BLESS=1`), pause, TCP |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Processes run on one thread under `--dap` (`Config::threads = 1`, the CLI warns `W0109` for more) | A stop inside a process is mid-delta; one thread makes every stop a defined point and sessions identical at any `--threads` | ir.md 3.4, `docs/decisions.md` "DAP server" |
| DAP threads are processes: `threadId` = session process index + 1, named by `Elaborated::proc_name` | One debuggable sequential thing per process | plan `yavs-gui` debugging list |
| Frame id `threadId << 16 \| depth` (outermost 0), variables reference `frameId << 2 \| scope` (1 locals, 2 signals, 3 generics) | Stateless ids, valid for one stop | this file |
| Requests that need no stop (`initialize`, `launch`, `setBreakpoints`, `configurationDone`, `threads`, `pause`) are answered on the reader thread; the rest go to the simulation thread and are served only while stopped (`the simulation is running` otherwise) | The simulation thread is busy inside `SimControl::run_until` | this file |
| A line breakpoint is set on the outermost statements starting on the line, or the next line with any (`Design::line_stmts`); the response gives the actual line | Editors expect breakpoints to slide to code | this file |
| Pause sets both the session pause (a running process stops at its next statement) and the kernel `Interrupt` (between cycles); the kernel pause that follows a process stop is swallowed (`paused_in_process`), an untaken session pause cancelled (`DebugSession::cancel_pause`) | An endless loop inside a process must be pausable; one pause is one stop | this file |
| `yavs/runFor`/`yavs/runUntil`/`yavs/stepDelta`/`yavs/stepTimestep` are `SimControl::run_until`/`step_delta`/`step_timestep` in the run loop (`Run`); from a stop inside a process they are queued (`Shared::queued`) and the kernel paused, so they start once the cycle has ended (a delta step is then that end) | Nothing but `SimControl` drives the kernel; a process stop is mid-cycle | issue #324 |
| A stop during a run to a time does not cancel it: the run loop keeps the limit until it is reached, so `continue` resumes it | One rule for line, signal and pause stops; matches a `run` command resumed after a breakpoint | this file |
| Custom run requests answer, then send `continued` (thread of the stop, or 1) | A client marks the session running only for the standard resume requests | VS Code |
| Signal breakpoints are one `SignalBreak` observer, subscribed before the run, whose shared watch list (`Watches`) holds every leaf of every signal breakpoint, on any change; `setDataBreakpoints` and `yavs/setSignalBreakpoints` each replace their own set (`Shared::sig_bps`), answered on the reader thread | The interpreter's signal breakpoint and the event bus, no second watcher; VS Code resends all data breakpoints on every change and must not drop the custom ones |
| Values the user types (value breakpoints, `yavs/force`, `yavs/deposit`, `setVariable`) are parsed for the target's type by `yavs_std::value::literal` (scalars as `'VALUE` reads them, any unit of a physical type, string and bit string literals, aggregates) and compared or written as `Lit`; after a pause with hits, a change that matches no value breakpoint resumes the run | One value syntax, VHDL's, for every request; the Variables view writes values in it | issues #322, #323 |
| Force, deposit and release act on whole signals through `SimControl::force`/`deposit`/`release` (one call per kernel leaf); a request at a stop only queues them (`Shared::ops`), the run loop makes them before it runs on, so they take effect in the next cycle. From a stop inside a process the kernel is paused after the current cycle, as for a queued run, and that pause is no stop | Nothing but `SimControl` drives the kernel, and a process stop is mid-cycle; one path for both kinds of stops | issue #323 |
| `setVariable` on a signal is a deposit; on a local it writes the variable at once (`FrameView::set_value`, the whole value, composites keep their bounds); generics are refused | The Variables view's "Set Value" does what a user expects of each | issue #323 |
| Signal variables carry `__vscodeVariableMenuContext: "signal"` | VS Code's context key for the extension's force, deposit and release menu entries | VS Code |
| Conditions and hit conditions on data breakpoints are rejected (`verified: false`) | Not supported yet (#326); no silent drop | this file |
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
- Codes owned: `E0700`-`E0799`/`W0700`-`W0799` (in use: `E0701` connection cannot be opened).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-dap -p yavs --test dap
YAVS_BLESS=1 cargo test -p yavs --test dap      # rewrite tests/dap/*.expected; review the diff
yavs run --work=lib:out/lib tb --dap=4711       # then attach an editor to 127.0.0.1:4711
```

## 6. Open issues

#325 live waves and `--wave`/`--vhpi` with `--dap`, #326 breakpoint conditions, composite
variables, hierarchy browsing.
