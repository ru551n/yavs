# yavs CLI contract for VUnit

This is the part of the `yavs` command line that VUnit (`vunit/sim_if/yavs.py`,
branch `feature/rsim`) relies on. Anything not listed here is free to change.
A conforming fake lives in VUnit's `tests/acceptance/yavs/fake_bin/yavs`.

Conventions:

- Every option is one argv element in `--name=value` form (`-gNAME=VALUE` for
  generics). VUnit never passes `--name value` as two elements, so yavs must
  accept the `=` form; it may also accept the split form for humans.
- VUnit builds argv as a list and never goes through a shell. On Windows the
  list is joined with `subprocess.list2cmdline`, so yavs must split its command
  line with the standard MSVC rules (Rust's `std::env::args` does).
- Options may appear in any order before the positional arguments. If an option
  that takes one value is repeated, the last one wins (VUnit relies on this for
  `--exit-severity`, which it always appends after user flags).
- Response files: an argument `@FILE`, in any position and for every
  subcommand, is replaced in place by the arguments read from FILE:
  - FILE is UTF-8 (a leading BOM is ignored), one argument per line. Lines end
    in `\n` or `\r\n`; the terminator of the last line is optional.
  - Each line is one argument taken verbatim: no quoting, no escapes, no
    trimming of spaces, no comments. An empty line is an empty argument.
  - Expansion is not recursive: a line starting with `@` is a literal
    argument. An argument containing a line break cannot be written; VUnit
    raises an error for one.
  - A relative FILE is relative to the current directory. A missing or
    unreadable FILE is a usage error (exit 2).
  - VUnit always passes `yavs run @FILE` / `yavs elab @FILE`, with everything
    after the subcommand in `<test output>/yavs/args.rsp`. That keeps
    `runner_cfg` clear of the Windows 32K command line limit and quoting.
    `analyze` is called with plain arguments.

## 0. Errors: fail early, fail clearly

- Every `yavs` command stops at the first failed stage (option parsing,
  analysis, elaboration, simulation) and exits non-zero. It never continues to
  a later stage after an error and never exits 0 after printing an error.
- Errors are rustc-format diagnostics (code, primary span, notes) on stderr.
  Within one stage, analysis may report all independent errors before stopping.
- `--message-format=json` prints the same diagnostics as one JSON object per
  line on stderr, with at least: `level`, `code`, `message`, `spans`
  (`file`, `line`, `column`, `label`), `children` (notes/help) and `rendered`
  (the human text). VUnit may later pass it to surface diagnostics in its own
  report; until then it uses the human format.
- VUnit's side: it merges yavs's stderr into stdout and shows all of it, it
  never masks the exit code (a non-zero `analyze` is a compile failure, a
  non-zero `run`/`elab` fails the test even when `vunit_results` says done),
  and it raises a clear error before compiling anything when the executable is
  missing, `yavs --version` fails or cannot be parsed, or the version is older
  than the first one implementing this contract (`YAVS_MIN_VERSION` in
  `vunit/sim_if/yavs.py`).

## 1. Discovery

- Executable: `yavs` (`yavs.exe` on Windows), found on `PATH`, or in the
  directory named by `VUNIT_YAVS_PATH`.
- `yavs --version` exits 0 and prints, on the first line of stdout:

  ```
  yavs MAJOR.MINOR.PATCH[-PRERELEASE] [(anything)]
  ```

  e.g. `yavs 0.4.0-dev (1a2b3c4 2026-10-18)`. VUnit takes the first
  `MAJOR.MINOR[.PATCH]` it finds; the prerelease tag is ignored for ordering.
  The version is available to VUnit packages as the `yavs_version` marker.

VUnit's capability answers for yavs (`SimulatorInterface` hooks):

| Hook | Answer | Why |
|---|---|---|
| `supports_vhdl_contexts` | True | 2008 |
| `supports_vhdl_package_generics` | True | M3 |
| `supports_vhdl_call_paths` | True | 2019 `std.env` call paths, M3/M4 |
| `has_valid_exit_code` | True | see section 5 |
| `supports_vhpi` | True | M5 (`--vhpi`) |
| `supports_coverage` | False | not in the contract (section 8) |
| `supports_gui_flag` | True | waves opened in Surfer, section 6 |

## 2. Standards

`--std=2008` or `--std=2019`. Any other value is a usage error (exit 2). VUnit
raises an error itself for a project containing VHDL-93/2002 files.

- A project may mix 2008 and 2019 files. `analyze --std=STD` gives the
  standard of that one file (default 2008), and yavs records it per file in the
  library manifest. VUnit passes each file's own standard.
- `run`/`elab` re-analyze every file with its recorded standard. Their `--std`
  is optional and VUnit does not pass it: the whole design uses ONE `std`
  and `ieee` revision, the newest standard recorded for any file in the
  design's closure (not the top level unit's standard), and that revision
  selects the run-time semantics of the whole design (e.g. 64-bit `integer`
  in 2019, plan rule 7). `--std` on `run`/`elab` overrides it. A file using a
  declaration or construct that is not available under that revision fails
  early with `E1007`, naming the file and its recorded standard (exit 2),
  never a silent reinterpretation. A construct whose meaning changes under
  that revision is warning `W1008` (once per construct; `-Werror=W1008`
  escalates it). See `docs/design/sem.md` section 0.1.

## 3. Libraries

- A library is a directory. `--work=NAME:PATH` names the library being analyzed
  into (or the library holding the top level, for `run`/`elab`).
  `--map=NAME:PATH` maps any other library. VUnit passes `--map` for every
  library in the project, including the work library: a `--map` identical to
  `--work` must be accepted.
- A missing or empty directory is a valid empty library. `analyze` creates the
  directory (and its parents) on first use. VUnit does not run an init command;
  it only creates the parent of the library directory.
- `std` and `ieee` (and any other library yavs ships) are resolved by yavs
  itself, on demand from yavs's `lib/` tree (nothing is cached). A user `--map` or
  `--work` of `std` or `ieee` is an error (`E0102`), except under
  `yavs analyze --bootstrap`, which is how yavs's own `lib/std` and `lib/ieee` are
  analyzed (tools/corpus `yavs-lib-*`): the mapped `std`/`ieee` directories then
  replace the vendored tree for that analysis.
- Library names are case-insensitive VHDL identifiers; paths are used verbatim
  and may contain spaces and, on Windows, drive letters (`C:\x`): split
  `NAME:PATH` at the first colon only.
- Concurrency (plan rule 14): `run` and `elab` never write into any library
  directory, so any number of them may run at once against the same libraries.
  `analyze` must be safe when several processes analyze into the same library
  at the same time (two `run.py` invocations, or a future parallel compile in
  VUnit). yavs does this with automatic library locking like NVC's: each library
  directory has a `_YAVS_LIB` lock file held exclusively by `analyze` while it
  updates the manifest (written to a temp file, then atomically renamed);
  `run`/`elab` hold it shared only while reading the manifest. Read-only
  library directories never fail: shared lock via a read-only open, or no lock
  if the file cannot be opened at all. VUnit needs no locking of its own.

## 4. `yavs analyze`

```
yavs analyze --std=FILE_STD --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
             [--bootstrap] [yavs.global_flags] [yavs.a_flags] FILE...
```

- One or more files per call, in dependency order (VUnit `--batch-compile` /
  `--parallel-compile`: directly as several arguments, or as one `@filelist`
  response file with a file per line, section 0). All files get the same options
  and `--std`. They are analyzed in the given order by one analysis session: each
  file is parsed once and sees the units of the files before it, even before the
  manifest is read again. The work library's exclusive lock is held for the
  whole call, and each successful file is recorded (atomic manifest rename) as
  soon as it is done, so concurrent calls into the same library serialize per
  call and never lose each other's files.
- Exit 0: every file was analyzed and recorded in the work library. Warnings go
  to stderr/stdout and do not change the exit code (unless `-Werror`).
- Exit 1: a file has errors. The call stops at the first failing file (fail
  early); files before it stay recorded, the failing one and the rest are not
  analyzed. With more than one file, stderr names the failing file and lists the
  files analyzed and recorded before it.
- `--bootstrap` allows `std`/`ieee` as `--work`/`--map` (section 3).
- Exit 2: usage error (bad option, unknown standard, unmappable library).
- Exit 3: internal compiler error.
- VUnit shows the combined stdout/stderr to the user on failure, so the default
  rustc-style human format is the right one; VUnit does not request
  `--message-format=json` (section 0).
- Options from plan rule 13 that users will put in `yavs.a_flags`:
  `--relaxed`, `-W<code>`/`-Wno-<code>`/`-Werror`.

## 5. `yavs run` and `yavs elab`

```
yavs run  [--std=STD] --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
          [--ieee-warnings=off] [yavs.global_flags] [yavs.elab_flags]
          [yavs.sim_flags] --exit-severity=LEVEL [--wave=FILE.fst]
          TOP [-gNAME=VALUE ...]
yavs elab [--std=STD] --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
          [yavs.global_flags] [yavs.elab_flags] TOP [-gNAME=VALUE ...]
```

VUnit passes everything after `run`/`elab` in a response file (Conventions).

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
  cases) and contains spaces, `:`, `,`, `/`, `\` and `.`: yavs must not split or
  unescape it. A `-g` for a generic the top level does not have is a warning,
  not an error. Also accept `-gNAME=VALUE` after `TOP` (VUnit puts them there).
- `--exit-severity=LEVEL` (`note|warning|error|failure`): an assertion or
  report at or above LEVEL stops the simulation at the end of the current delta
  and gives exit 1. Default `failure` (user decision 2026-10-18, as NVC, GHDL and
  Questa): an `error` report is printed and the simulation continues, and the run
  still exits 1 at the end because an `error` (or higher) was reported. yavs's own
  runtime errors (bound check, overflow, null access, delta limit, internal) stop
  at once whatever the level. VUnit passes its `vhdl_assert_stop_level` sim option.
- `--ieee-warnings=off`: suppress the warnings raised inside `ieee` packages
  (VUnit `disable_ieee_warnings`).
- `--wave=FILE.fst`: write an FST of the whole design (M7). Flushed on normal
  end, on stop, and on SIGINT.
- `--stop-time=TIME` (e.g. `10ms`), `--threads=N`, `--shuffle`, `--load=LIB`,
  `--vhpi=LIB[:ENTRY]`, `--dap=PORT`: not passed by VUnit, used through
  `yavs.sim_flags` or simulator hooks (the VUnit python bridge adds `--load`).
- Threads: yavs defaults to `--threads=1`. VUnit already runs `-p N`
  simulations in parallel and never passes a thread count; a user who wants
  parallel process evaluation inside one simulation opts in with
  `yavs.sim_flags`, e.g. `["--threads=4"]`.
- Exit codes:
  - 0: simulation ended without failure: event starvation, `std.env.finish`
    or `std.env.stop` with no status or status 0, or `--stop-time` reached.
  - 1: simulation failure: an assertion/report at or above `--exit-severity`,
    or of severity `error` or `failure` at any point,
    `std.env.stop(N)`/`finish(N)` with N /= 0 (the status itself may be
    returned if it fits in 1..125; VUnit only tests zero/non-zero), a runtime
    error (bound check, null access, delta-cycle limit).
  - 2: usage, analysis-on-demand or elaboration error.
  - 3: internal error.
- The VUnit runner itself writes the `vunit_results` file (`test_start:NAME`
  lines, then `test_suite_done`) into the `output path` from `runner_cfg`, via
  textio. yavs needs no VUnit knowledge beyond correct textio and `std.env`.
- Standard output: `report`/`assert` messages go to stdout in the human format,
  one message per line group, flushed at least at every simulation time step
  so that VUnit's live output and timeouts work. `--color=always` forces ANSI
  colour on a pipe (VUnit passes it when it wants colour); without it yavs
  follows the TTY/`NO_COLOR` rules of plan `yavs-diag`.
- `--message-format=json` (section 0) is accepted by `run` and `elab` too, and
  covers runtime errors with their VHDL stacktrace; VUnit does not pass it today.

## 6. GUI and waves

VUnit `-g/--gui` (until `yavs-gui` exists): VUnit runs `yavs run ...
--wave=<output>/yavs/<entity>.fst`, then opens the file in the viewer:
`--viewer` argument, then `yavs.viewer.gui` sim option, then `surfer`, then
`gtkwave`. `yavs.viewer_script.gui` is passed to the viewer as
`--script FILE`. Only FST is written; VUnit warns and ignores `--viewer-fmt`
values other than `fst`.

Later: when `yavs-gui` lands, `-g` will run `yavs run --dap=0 ...` and start
`yavs-gui` attached to it. `--dap=0` makes yavs pick a free port, and its first
line on stderr is then exactly `yavs: DAP listening on 127.0.0.1:<port>`;
VUnit reads that line to connect the GUI. `--dap=PORT` with a fixed port
prints the same line.

## 7. Environment

VUnit sets no yavs environment variables. Simulator hooks from VUnit packages
can add `elab_flags`, `run_flags` and run environment; VUnit applies
`elab_flags` to both `run` and `elab`, and `run_flags` only to `run`.

## 8. Not covered yet

Coverage: not part of the VUnit contract; VUnit reports
`supports_coverage() == False`.
