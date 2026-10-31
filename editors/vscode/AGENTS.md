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
- `yavs: Attach` (`yavs.attach`, the dynamic configuration, and F5 with nothing configured, `Configurations`) needs no launch.json: port 4711, and an input box only if nothing listens there. The look at the port connects and closes, which the server takes for no session. Nothing is analyzed. The flow: `python run.py lib.tb.test --yavs-debug`, breakpoints, F5.
- `surfer` in a launch configuration is `--surfer[=HOST:PORT]`; the custom event `yavs/showSource` (Surfer's go to declaration) opens the file at the line.
- The Debug Console needs nothing here: it sends `evaluate` with context `repl`, and the server takes the console's commands that way (`run 10 ns`, `restart`, #904).
- Contributes the problem matcher `$yavs` for a task that runs yavs with `--message-format=short` (`yavs lint`, `yavs analyze`): a language server would be the way to live diagnostics, and yavs has none, so a task is the one surface that needs no code here.
- Does not: implement DAP (all of it is `yavs-dap`), highlight or lint VHDL.
- Plain JavaScript, no bundler, no runtime dependencies.

## 2. Map

| File | Responsibility |
|---|---|
| `package.json` | Manifest: `contributes.debuggers`, languages, breakpoints, snippets, `problemMatchers` |
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

CI (`.github/workflows/vscode.yml`, on changes under this directory or `crates/yavs-dap`) runs both tests against a debug build of the same commit's yavs and packages the .vsix. The extension is released with yavs, in the same release and with the same version (root `RELEASING.md`): `release.yml` calls `vscode.yml` with the Linux x86_64 archive of its run and attaches the .vsix; `tools/release/bump.py` sets `package.json` and adds the `CHANGELOG.md` section. Marketplace and Open VSX publishing is not wired (needs tokens; the hook is described in `release.yml`). Run both tests locally after changing this directory or `yavs-dap` (a newer yavs than the released one):

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
