# rsim CLI contract for VUnit

This is the part of the `rsim` command line that VUnit (`vunit/sim_if/rsim.py`,
branch `feature/rsim`) relies on. Anything not listed here is free to change.
A conforming fake lives in VUnit's `tests/acceptance/fake_rsim/rsim`.

Conventions:

- Every option is one argv element in `--name=value` form (`-gNAME=VALUE` for
  generics). VUnit never passes `--name value` as two elements, so rsim must
  accept the `=` form; it may also accept the split form for humans.
- VUnit builds argv as a list and never goes through a shell. On Windows the
  list is joined with `subprocess.list2cmdline`, so rsim must split its command
  line with the standard MSVC rules (Rust's `std::env::args` does).
- Options may appear in any order before the positional arguments. If an option
  that takes one value is repeated, the last one wins (VUnit relies on this for
  `--exit-severity`, which it always appends after user flags).

## 0. Errors: fail early, fail clearly

- Every `rsim` command stops at the first failed stage (option parsing,
  analysis, elaboration, simulation) and exits non-zero. It never continues to
  a later stage after an error and never exits 0 after printing an error.
- Errors are rustc-format diagnostics (code, primary span, notes) on stderr.
  Within one stage, analysis may report all independent errors before stopping.
- `--message-format=json` prints the same diagnostics as one JSON object per
  line on stderr, with at least: `level`, `code`, `message`, `spans`
  (`file`, `line`, `column`, `label`), `children` (notes/help) and `rendered`
  (the human text). VUnit may later pass it to surface diagnostics in its own
  report; until then it uses the human format.
- VUnit's side: it merges rsim's stderr into stdout and shows all of it, it
  never masks the exit code (a non-zero `analyze` is a compile failure, a
  non-zero `run`/`elab` fails the test even when `vunit_results` says done),
  and it raises a clear error before compiling anything when the executable is
  missing, `rsim --version` fails or cannot be parsed, or the version is older
  than the first one implementing this contract (`RSIM_MIN_VERSION` in
  `vunit/sim_if/rsim.py`).

## 1. Discovery

- Executable: `rsim` (`rsim.exe` on Windows), found on `PATH`, or in the
  directory named by `VUNIT_RSIM_PATH`.
- `rsim --version` exits 0 and prints, on the first line of stdout:

  ```
  rsim MAJOR.MINOR.PATCH[-PRERELEASE] [(anything)]
  ```

  e.g. `rsim 0.4.0-dev (1a2b3c4 2026-10-18)`. VUnit takes the first
  `MAJOR.MINOR[.PATCH]` it finds; the prerelease tag is ignored for ordering.
  The version is available to VUnit packages as the `rsim_version` marker.

VUnit's capability answers for rsim (`SimulatorInterface` hooks):

| Hook | Answer | Why |
|---|---|---|
| `supports_vhdl_contexts` | True | 2008 |
| `supports_vhdl_package_generics` | True | M3 |
| `supports_vhdl_call_paths` | True | 2019 `std.env` call paths, M3/M4 |
| `has_valid_exit_code` | True | see section 5 |
| `supports_vhpi` | True | M5 (`--vhpi`) |
| `supports_coverage` | False | until M9 defines `--cov` and a merge command |
| `supports_gui_flag` | True | waves opened in Surfer, section 6 |

## 2. Standards

`--std=2008` or `--std=2019`. Default 2008. Any other value is a usage error
(exit 2). VUnit raises an error itself for a project containing VHDL-93/2002
files, and for a project mixing 2008 and 2019 files (one `--std` is used for
the whole project, on both `analyze` and `run`/`elab`).

## 3. Libraries

- A library is a directory. `--work=NAME:PATH` names the library being analyzed
  into (or the library holding the top level, for `run`/`elab`).
  `--map=NAME:PATH` maps any other library. VUnit passes `--map` for every
  library in the project, including the work library: a `--map` identical to
  `--work` must be accepted.
- A missing or empty directory is a valid empty library. `analyze` creates the
  directory (and its parents) on first use. VUnit does not run an init command;
  it only creates the parent of the library directory.
- `std` and `ieee` (and any other library rsim ships) are resolved by rsim
  itself. A user `--map` of `std` or `ieee` is an error.
- Library names are case-insensitive VHDL identifiers; paths are used verbatim
  and may contain spaces and, on Windows, drive letters (`C:\x`): split
  `NAME:PATH` at the first colon only.
- Concurrency (plan rule 14): `run` and `elab` never write into any library
  directory, so any number of them may run at once against the same libraries.
  `analyze` must be safe when several processes analyze into the same library
  at the same time (two `run.py` invocations, or a future parallel compile in
  VUnit). rsim does this with automatic library locking like NVC's: each library
  directory has a `_RSIM_LIB` lock file held exclusively by `analyze` while it
  updates the manifest (written to a temp file, then atomically renamed);
  `run`/`elab` hold it shared only while reading the manifest. Read-only
  library directories never fail: shared lock via a read-only open, or no lock
  if the file cannot be opened at all. VUnit needs no locking of its own.

## 4. `rsim analyze`

```
rsim analyze --std=STD --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
             [rsim.global_flags] [rsim.a_flags] FILE
```

- VUnit passes exactly one file per call, in dependency order.
- Exit 0: the file was analyzed and recorded in the work library. Warnings go
  to stderr/stdout and do not change the exit code (unless `-Werror`).
- Exit 1: the file has errors; nothing is recorded for that file.
- Exit 2: usage error (bad option, unknown standard, unmappable library).
- Exit 3: internal compiler error.
- VUnit shows the combined stdout/stderr to the user on failure, so the default
  rustc-style human format is the right one; VUnit does not request
  `--message-format=json` (section 0).
- Options from plan rule 13 that users will put in `rsim.a_flags`:
  `--relaxed`, `-W<code>`/`-Wno-<code>`/`-Werror`.

## 5. `rsim run` and `rsim elab`

```
rsim run  --std=STD --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
          [--ieee-warnings=off] [rsim.global_flags] [rsim.elab_flags]
          [rsim.sim_flags] --exit-severity=LEVEL [--wave=FILE.fst]
          TOP [-gNAME=VALUE ...]
rsim elab --std=STD --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
          [rsim.global_flags] [rsim.elab_flags] TOP [-gNAME=VALUE ...]
```

- `run` elaborates and simulates in one process (no elaboration artefact is
  saved: no-cache design). `elab` elaborates only, for VUnit's
  `--elaborate` mode, and has the same exit codes as `run` minus the
  simulation ones.
- `TOP` is `ENTITY`, `ENTITY.ARCH` or `CONFIGURATION`, looked up in the work
  library. VUnit always passes `ENTITY.ARCH` or a configuration name. No
  library prefix.
- Generics: `-gNAME=VALUE`, one argv element each, split at the first `=`.
  `NAME` is case-insensitive. `VALUE` is parsed according to the generic's type
  with the normal value parser (plan rule 4); for a `string` generic it is the
  raw text, taken verbatim, without VHDL quotes. It can be long (VUnit's
  `runner_cfg` is several hundred bytes and grows with the number of test
  cases) and contains spaces, `:`, `,`, `/`, `\` and `.`: rsim must not split or
  unescape it. A `-g` for a generic the top level does not have is a warning,
  not an error. Also accept `-gNAME=VALUE` after `TOP` (VUnit puts them there).
- `--exit-severity=LEVEL` (`note|warning|error|failure`): an assertion or
  report at or above LEVEL stops the simulation at the end of the current delta
  and gives exit 1. Default `error`. VUnit passes its
  `vhdl_assert_stop_level` sim option.
- `--ieee-warnings=off`: suppress the warnings raised inside `ieee` packages
  (VUnit `disable_ieee_warnings`).
- `--wave=FILE.fst`: write an FST of the whole design (M7). Flushed on normal
  end, on stop, and on SIGINT.
- `--stop-time=TIME` (e.g. `10ms`), `--threads=N`, `--shuffle`, `--load=LIB`,
  `--vhpi=LIB[:ENTRY]`, `--dap=PORT`: not passed by VUnit, used through
  `rsim.sim_flags` or simulator hooks (the VUnit python bridge adds `--load`).
- Exit codes:
  - 0: simulation ended without failure: event starvation, `std.env.finish`
    or `std.env.stop` with no status or status 0, or `--stop-time` reached.
  - 1: simulation failure: an assertion/report at or above `--exit-severity`,
    `std.env.stop(N)`/`finish(N)` with N /= 0 (the status itself may be
    returned if it fits in 1..125; VUnit only tests zero/non-zero), a runtime
    error (bound check, null access, delta-cycle limit).
  - 2: usage, analysis-on-demand or elaboration error.
  - 3: internal error.
- The VUnit runner itself writes the `vunit_results` file (`test_start:NAME`
  lines, then `test_suite_done`) into the `output path` from `runner_cfg`, via
  textio. rsim needs no VUnit knowledge beyond correct textio and `std.env`.
- Standard output: `report`/`assert` messages go to stdout in the human format,
  one message per line group, flushed at least at every simulation time step
  so that VUnit's live output and timeouts work. `--color=always` forces ANSI
  colour on a pipe (VUnit passes it when it wants colour); without it rsim
  follows the TTY/`NO_COLOR` rules of plan `rsim-diag`.
- `--message-format=json` (section 0) is accepted by `run` and `elab` too, and
  covers runtime errors with their VHDL stacktrace; VUnit does not pass it today.

## 6. GUI and waves

VUnit `-g/--gui` (until `rsim-gui` exists): VUnit runs `rsim run ...
--wave=<output>/rsim/<entity>.fst`, then opens the file in the viewer:
`--viewer` argument, then `rsim.viewer.gui` sim option, then `surfer`, then
`gtkwave`. `rsim.viewer_script.gui` is passed to the viewer as
`--script FILE`. Only FST is written; VUnit warns and ignores `--viewer-fmt`
values other than `fst`.

Later: when `rsim-gui` lands, `-g` will run `rsim run --dap=PORT ...` and start
`rsim-gui` attached to it; that needs a contract addition for the port
handshake (rsim printing `dap listening on 127.0.0.1:PORT` to stdout, or
`--dap=0` choosing a free port and printing it).

## 7. Environment

VUnit sets no rsim environment variables. Simulator hooks from VUnit packages
can add `elab_flags`, `run_flags` and run environment; VUnit applies
`elab_flags` to both `run` and `elab`, and `run_flags` only to `run`.

## 8. Not covered yet

Coverage (`--cov`, merge): VUnit reports `supports_coverage() == False` until
M9 defines `rsim run --cov=FILE` and `rsim cov merge -o OUT FILES...`.
