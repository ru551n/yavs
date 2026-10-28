# yavs VHDL debugger for VS Code

Debug VHDL testbenches with the [yavs](https://github.com/ru551n/yavs) simulator from VS Code's
Run and Debug view. The extension contributes a `yavs` debugger that analyzes your sources,
starts `yavs run --dap=stdio` and connects to it over the Debug Adapter Protocol. It also
registers the `vhdl` language for `.vhd` and `.vhdl` files so that breakpoints can be set
without another VHDL extension (it does not highlight or lint VHDL).

## Install

The extension is built from the yavs source tree (it is plain JavaScript, no bundler). yavs must
be on `PATH`, or set `yavsPath` in the launch configuration.

```
cd editors/vscode
npx @vscode/vsce package --out yavs-debug.vsix
code --install-extension yavs-debug.vsix
```

To try it without installing, open `editors/vscode` in VS Code and press F5. It needs VS Code
1.80 or newer.

## Launch configuration

Add a configuration to the testbench workspace's `.vscode/launch.json` (the snippet "yavs: Debug
testbench" inserts one; `sample/` is a complete workspace):

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
join it. Use the configuration (the snippet "yavs: Attach to yavs" inserts it):

```json
{ "type": "yavs", "request": "launch", "name": "Attach to yavs", "port": 4711 }
```

Set breakpoints, start the configuration with F5, and the run continues to the first one.

## What you can do

- Line breakpoints, with a condition (a VHDL boolean expression over locals and visible
  signals) and a hit condition (`3`, `>= 3`, `% 3`). A breakpoint on a line without a statement
  moves to the next line that has one. Function calls and attributes in conditions are not
  supported: such a breakpoint shows as unverified with the reason.
- Continue, pause, step over, into and out. The call stack lists one thread per process.
- The Variables view shows locals, the signals and ports around the process, the generics of its
  instance and a Hierarchy scope. Records expand by field, large arrays in pages. "Set Value"
  writes a variable at once and deposits a signal.
- Hover and the Watch view evaluate a local, a signal visible from the process, or a signal by
  path (`:tb:u1:s` or `tb.u1.s`).
- "Break on Value Change" on a signal in the Variables view stops after the cycle in which it
  changed.
- Every stop shows the simulation time and delta. Reports go to the Debug Console.

Commands in the palette, available only while a `yavs` session runs:

| Command | Effect |
|---|---|
| yavs: Run for... | run for a time such as `10 ns`, then stop |
| yavs: Run until... | run to a time |
| yavs: Step to the next delta | stop at the end of the current cycle |
| yavs: Step to the next time step | run the next cycle and all deltas at that time |
| yavs: Break when a signal equals... | stop when the signal takes a VHDL literal of its type |
| yavs: Clear signal value breakpoints | remove those |
| yavs: Force a signal..., Deposit a signal value..., Release a forced signal... | also in the context menu of a signal in the Variables view; the target may be a field, an element or a slice |

Values are VHDL literals of the target's type: `'1'`, `42`, `16#FF#`, `"01ZX"`, `x"A5"`, `10 ns`,
aggregates. A force or deposit takes effect in the next simulation cycle.

## Waveforms

The debugger runs with the same recorder as `--wave`. Add `--wave=out.fst` to `args` (or to the
`yavs run` of an attach session) and open the file in a waveform viewer; under the debugger yavs
writes the file out at every stop, so it holds the completed time steps up to the stop. The
server can also stream waves to a client (`yavs/waveSubscribe`); this extension does not use it.

## Tests

The extension is not built or tested in CI (there is no Node there). After changing this
directory or the DAP server, run both tests, which are described in `AGENTS.md`:
`test/client.js` drives `yavs run --dap=stdio` the way VS Code does, and `test/vscode/run.js` runs
a real VS Code headless on `sample/`.
