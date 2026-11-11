# yavs VHDL debugger for VS Code

Debug VHDL testbenches with the [yavs](https://github.com/ru551n/yavs) simulator from VS Code's
Run and Debug view. The extension contributes a `yavs` debugger that attaches to a test waiting
on a port, over the Debug Adapter Protocol, with no configuration. It also
registers the `vhdl` language for `.vhd` and `.vhdl` files so that breakpoints can be set
without another VHDL extension (it does not highlight or lint VHDL).

## Debug a VUnit test

No configuration, nothing to analyze:

```
python run.py lib.tb.test_name --yavs-debug
```

Set breakpoints in your source files and press F5, or run "yavs: Attach" from the command palette. The extension
attaches to port 4711, and asks for a port only if nothing listens there. Breakpoints in your own files work with
VUnit's location and check preprocessing on, and the stack shows your files. The Debug Console takes the yavs
console's commands (`run 10 ns`, `restart`, `vunit_restart`).

A step-by-step tutorial with runnable files (`docs/examples/fifo-debug`) walks through a failing test with this
extension, Surfer and the console: [debugger tutorial](https://yavs.readthedocs.io/en/latest/debugger-tutorial.html).
It needs yavs 0.12.0 or later.

## Install

yavs 0.19.1 or newer must be on `PATH` of the test run. The
extension needs VS Code 1.80 or newer.

Download `yavs-debug-<version>.vsix` from the
[releases of yavs](https://github.com/ru551n/yavs/releases) (yavs releases from 0.12.0 on carry the
extension of the same version, 0.1.0 is the release "yavs VS Code debugger 0.1.0"; `SHA256SUMS`
is next to it) and install it:

```
code --install-extension yavs-debug-<version>.vsix
```

Or build it from the yavs source tree (plain JavaScript, no bundler):

```
cd editors/vscode
npx @vscode/vsce package --out yavs-debug.vsix
code --install-extension yavs-debug.vsix
```

To try it without installing, open `editors/vscode` in VS Code and press F5.

## Launch configuration (optional)

A test run needs none of this. To start a run without a test runner, the extension analyzes your sources and starts
`yavs run --dap=stdio`, from a configuration in the testbench workspace's `.vscode/launch.json` (the snippet "yavs: Debug
testbench" inserts one; `sample/` is a complete workspace). It takes generics and arguments, and `yavsPath` names the
executable. A fixed port for attaching is in the next section.

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "yavs",
      "request": "launch",
      "name": "Debug tb",
      "top": "tb",
      "work": "work:${workspaceFolder}/.yavs/work",
      "sources": ["${workspaceFolder}/pkg.vhd", "${workspaceFolder}/tb.vhd"],
      "generics": { "WIDTH": "8" },
      "args": ["--stop-time=1ms"]
    }
  ]
}
```

| Attribute | Meaning |
|---|---|
| `top` | required: the top level, `ENTITY`, `ENTITY.ARCH` or a configuration in the work library |
| `work` | the work library as `NAME:PATH` (default `work:.yavs/work`) |
| `libraries` | more libraries as `NAME:PATH`, passed as `--map` |
| `surfer` | `true` starts Surfer from `PATH`, `HOST:PORT` connects to a running one; the cursor follows the stops, and its "go to declaration" opens the file |
| `sources` | files analyzed into the work library, in order, before every session; empty means you analyze yourself |
| `std` | `2008` or `2019`, passed as `--std` to analyze and run |
| `generics` | generics of the top level, passed as `-gNAME=VALUE` |
| `args` | more arguments for `yavs run`, such as `--stop-time=1ms` |
| `stopOnEntry` | stop at the first statement (default `false`) |
| `yavsPath` | the yavs executable (default `yavs`) |
| `cwd` | working directory of yavs (default: the first workspace folder) |
| `port` | attach to a running `yavs run --dap=PORT` instead of starting yavs |

Limitation: `std` accepts only `2008` or `2019`, so VHDL-1993 code (`--std=93`) cannot be debugged from the extension; analyze it yourself and attach to a `yavs run --dap=PORT` started by hand instead.

The design on the command line is the design that is debugged; `args` must not contain `--cov`
(it cannot be combined with the debugger yet) and the debugger runs the design on one thread, so
`--threads` is ignored.

## Attach to a run you started

`yavs run --dap=PORT` listens on `127.0.0.1:PORT`, prints `yavs: DAP listening on
127.0.0.1:PORT` and `yavs: waiting for a debugger on port PORT` on standard error, and holds the
simulation until a client attaches. This lets a script or a test runner start the run and VS Code
join it. Run "yavs: Attach" from the command palette, or use this configuration (the snippet "yavs: Attach" inserts it):

```json
{ "type": "yavs", "request": "attach", "name": "yavs: Attach", "port": 4711 }
```

Set breakpoints, start the configuration with F5, and the run continues to the first one.

## What you can do

- Line breakpoints, with a condition (a VHDL boolean expression over locals and visible
  signals) and a hit condition (`3`, `>= 3`, `% 3`). A breakpoint on a line without a statement
  moves to the next line that has one. Attributes and function calls other than `to_integer` in
  conditions are not supported: such a breakpoint shows as unverified with the reason.
- Continue, pause, step over, into and out. The call stack lists one thread per process.
- The Variables view shows locals, the signals and ports around the process, the generics of its
  instance and a Hierarchy scope. Records expand by field, large arrays in pages. "Set Value"
  writes a variable at once and deposits a signal.
- Hover and the Watch view evaluate a local, a signal visible from the process, or a signal or
  process variable by dotted path (`tb.u1.s`; `:tb:u1:s` is accepted too). A vector shows on one
  line; read one bit with `dout(3)`. A name that does not exist shows an error with the nearest name:
  VS Code lists a Watch entry before it asks the debugger, so the entry stays until you remove it.
- The Debug Console takes the console's commands (`help` lists them, names in bold). Tab takes what
  the entries in the list have in common and, with no list open, opens it. After a space with an
  empty word the list has nothing in common, and a second Tab takes the selected entry.
- "Break on Value Change" on a signal in the Variables view stops after the cycle in which it
  changed.
- Every stop shows the simulation time and delta. Reports go to the Debug Console.
- The Breakpoints view has filters that stop at a report of a severity: Assertion: failure, error, warning, Report: note.

Commands in the palette, available only while a `yavs` session runs:

| Command | Effect |
|---|---|
| yavs: Run for... | run for a time such as `10 ns`, then stop |
| yavs: Run until... | run to a time |
| yavs: Step to the next delta | stop at the end of the current cycle |
| yavs: Step to the next time step | run the next cycle and all deltas at that time |
| yavs: Break when a signal equals... | stop when the signal takes a VHDL literal of its type |
| yavs: Clear signal value breakpoints | remove those |
| yavs: Show Drivers | in the context menu of a signal in the Variables view: each driver, its value and source line, in the Debug Console |
| yavs: Find Signals or Variables... | search the hierarchy with a pattern; what you pick is added to the wave, or else examined |
| Add to Wave | in the context menu of the Variables and Watch views and of a selection or word in a VHDL file, in a session with Surfer: adds the signal or process variable, with its history since time 0 |
| yavs: Force a signal..., Deposit a signal value..., Release a forced signal... | also in the context menu of a signal in the Variables view; the target may be a field, an element or a slice |

Values are VHDL literals of the target's type: `'1'`, `42`, `16#FF#`, `"01ZX"`, `x"A5"`, `10 ns`,
aggregates; a bit-like target needs no quotes (`1`, `0101`), and an integer's `1` stays 1. A force or
deposit takes effect in the next simulation cycle. A force holds what readers of the signal see; a
release ends any force.

## Waveforms

Set `surfer` in the launch configuration (`--surfer` for an attach session, `--yavs-surfer` with a test runner) to see the
waves in [Surfer](https://surfer-project.org) 0.6.0 or later while the session runs: the cursor follows the stops, and its
"go to declaration" opens the file. See [waveforms](https://yavs.readthedocs.io/en/latest/waveforms.html).

Without Surfer, the debugger runs with the same recorder as `--wave`. Add `--wave=out.fst` to `args` (or to the
`yavs run` of an attach session) and open the file in a waveform viewer; under the debugger yavs
writes the file out at every stop, so it holds the completed time steps up to the stop. The
server can also stream waves to a client (`yavs/waveSubscribe`); this extension does not use it.

## Tests

CI (`.github/workflows/vscode.yml`) runs both tests and packages the extension on every change to
this directory, against yavs built from the same commit; a release tests it against the release
archive. They are described in
`AGENTS.md`: `test/client.js` drives `yavs run --dap=stdio` the way VS Code does, and
`test/vscode/run.js` runs a real VS Code headless on `sample/`.

## Problems view

The extension contributes the problem matcher `$yavs`: a task that runs yavs with `--message-format=short`
puts its diagnostics in the Problems view and in the editor, for example `yavs lint`:

```json
{
  "label": "yavs lint",
  "type": "shell",
  "command": "yavs lint --message-format=short --recursive src",
  "problemMatcher": "$yavs"
}
```
