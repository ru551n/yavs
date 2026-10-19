# AGENTS.md: editors/vscode

Read the root `AGENTS.md` and `crates/yavs-dap/AGENTS.md` first. This is the VS Code extension
that contributes the `yavs` debugger (user requirement, 2026-10-19: the DAP server must work in
VS Code).

## 1. Purpose and boundaries

- Contributes debugger type `yavs` (launch attributes: `top`, `work`, `libraries`, `sources`,
  `std`, `generics`, `args`, `stopOnEntry`, `yavsPath`, `cwd`, `port`), the `vhdl` language id
  for `.vhd`/`.vhdl` (so breakpoints can be set without another VHDL extension) and breakpoints
  for it. A `DebugAdapterDescriptorFactory` runs `yavs analyze` on `sources`, then starts
  `yavs run ... --dap=stdio TOP`, or attaches to `port`. Commands (palette, only in a `yavs`
  session) send the server's custom requests: run for/until a time, next delta, next time step,
  signal value breakpoints (kept per session in the extension, since the request replaces the
  set), force, deposit and release of a signal (also in the Variables view's context menu of a
  signal; called with `{signal, value, mode}` they ask nothing). "Break on Value Change" is VS Code's own (DAP data breakpoints).
- Does not: implement DAP (all of it is `yavs-dap`), highlight or lint VHDL.
- Plain JavaScript, no bundler, no runtime dependencies.

## 2. Map

| File | Responsibility |
|---|---|
| `package.json` | Manifest: `contributes.debuggers`, languages, breakpoints, snippets |
| `extension.js` | The adapter descriptor factory (`runArgs`, `libArgs`) and the commands (`custom`) |
| `sample/` | A testbench workspace with `.vscode/launch.json` (used by the tests) |
| `test/client.js` | A VS Code style session through the DAP test client against `yavs run --dap=stdio`: data breakpoints, `setVariable`, force and release, a deposit of an element, conditional line breakpoints, a paged array, the Hierarchy scope and `yavs/hierarchy`, live waves (`yavs/waveSubscribe`, `yavs/waves` events, values decoded from base64 by the hierarchy's `encoding`) |
| `test/vscode/run.js`, `suite.js` | A real VS Code (headless) on a copy of `sample/`: breakpoints with conditions VS Code sends, frame, scopes, an array paged as VS Code asks, `setVariable`, the force and release commands, the delta step command, a signal value breakpoint, continue to the end |

## 3. Design decisions

| Decision | Why |
|---|---|
| The extension starts yavs in stdio mode; `port` only attaches | VS Code owns the adapter's lifetime; no port to pick or leak |
| `sources` analyzed before every session by the extension | One F5 from an edited file to a session; the library stays an implementation detail |
| The launch arguments are turned into the command line; `yavs-dap` reads only `stopOnEntry` | One contract for the design (the command line), `docs/decisions.md` "DAP server" |

## 4. How to work here

Not in CI (no Node there). Run both tests after changing this directory or `yavs-dap`:

```sh
npm install --prefix ~/.cache/<dir>/npm @vscode/debugadapter-testsupport @vscode/test-electron @vscode/vsce
export NODE_PATH=~/.cache/<dir>/npm/node_modules
node test/client.js path/to/target/debug/yavs
node test/vscode/run.js path/to/target/debug/yavs [path/to/vscode/executable]
npx @vscode/vsce package --out yavs-debug.vsix
```

`test/vscode/run.js` passes `--ozone-platform=headless`, so no window opens; without an executable
path it downloads VS Code into `.vscode-test/`. Give it the VS Code binary itself (on Linux
packages `/usr/share/code/code`), not the `code` command: that wrapper returns at once with exit
status 0 and runs no test.
