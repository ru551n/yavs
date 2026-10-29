# yavs CLI contract for external test runners

This is the part of the `yavs` command line that an external test runner relies on. Anything not listed here is free to change.

Conventions:

- Every option is one argv element in `--name=value` form (`-gNAME=VALUE` for
  generics). The test runner never passes `--name value` as two elements, so yavs must
  accept the `=` form; it may also accept the split form for humans.
- The test runner builds argv as a list and never goes through a shell. On Windows the
  list is joined with `subprocess.list2cmdline`, so yavs must split its command
  line with the standard MSVC rules (Rust's `std::env::args` does).
- Options may appear in any order before the positional arguments. If an option
  that takes one value is repeated, the last one wins (the test runner relies on this for
  `--exit-severity`, which it always appends after user flags).
- Response files: an argument `@FILE`, in any position and for every
  subcommand, is replaced in place by the arguments read from FILE:
  - FILE is UTF-8 (a leading BOM is ignored), one argument per line. Lines end
    in `\n` or `\r\n`; the terminator of the last line is optional.
  - Each line is one argument taken verbatim: no quoting, no escapes, no
    trimming of spaces, no comments. An empty line is an empty argument.
  - Expansion is not recursive: a line starting with `@` is a literal
    argument. An argument containing a line break cannot be written; the test runner
    raises an error for one.
  - A relative FILE is relative to the current directory. A missing or
    unreadable FILE is a usage error (exit 2).
  - The test runner always passes `yavs run @FILE` / `yavs elab @FILE`, with everything
    after the subcommand in `<test output>/yavs/args.rsp`. That keeps
    `runner_cfg` clear of the Windows 32K command line limit and quoting.
    `analyze` is called with plain arguments.

## 0. Errors: fail early, fail clearly

- Every `yavs` command stops at the first failed stage (option parsing,
  analysis, elaboration, simulation) and exits non-zero. It never continues to
  a later stage after an error and never exits 0 after printing an error.
- Errors are diagnostics (code, primary span, labels, notes, help) on stderr.
  Within one stage, analysis may report all independent errors before stopping.
- `--message-format=json` prints the same diagnostics as one JSON object per
  line on stderr, with at least: `level`, `code` (plain string, e.g. `"E0002"`),
  `message`, `spans` (`file`, `byte_start`, `byte_end`, `line_start`,
  `column_start`, `line_end`, `column_end`, `primary`, `label`), `children`
  (notes then help, each `{level, message, spans, children, rendered}`) and
  `rendered` (the human text, without colour). Names follow the JSON diagnostics of common compilers.
  The test runner may later pass it to surface diagnostics in its own
  report; until then it uses the human format. `--message-format=short` prints one line per
  diagnostic (`FILE:LINE:COLUMN: error[CODE]: message`). In the JSON and short formats the
  note that names the failing file of a batch and the files recorded before it (section 4) is a note
  diagnostic too (`"level":"note"`, `"code":""`; the short format has the recorded files in its one
  line), so the JSON stream is nothing but JSON objects, one per line (#839).
- The test runner's side: it merges yavs's stderr into stdout and shows all of it, it
  never masks the exit code (a non-zero `analyze` is a compile failure, a
  non-zero `run`/`elab` fails the test even when `vunit_results` says done),
  and it raises a clear error before compiling anything when the executable is
  missing, `yavs --version` fails or cannot be parsed, or the version is older
  than the first one implementing this contract (`YAVS_MIN_VERSION` in
  the test runner).

## 1. Discovery

- Executable: `yavs` (`yavs.exe` on Windows), found on `PATH`, or in the
  directory named by `VUNIT_YAVS_PATH`.
- `yavs --version` exits 0 and prints, on the first line of stdout:

  ```
  yavs MAJOR.MINOR.PATCH[-PRERELEASE] [(anything)]
  ```

  e.g. `yavs 0.4.0-dev (1a2b3c4 2026-10-18)`. The details say whether the compiled tier is
  built in: `yavs 0.8.0 (compiled tier: llvm)` or `yavs 0.9.0 (interpreter only, no compiled
  tier)` (a build with neither `llvm` nor `clift` fails `--tier=auto` and `--tier=llvm` with E0120;
  a build with the `clift` feature says `compiled tier: cranelift`, or `llvm, cranelift` with
  both, and runs `--tier=auto` with Cranelift when `llvm` is not built in). The release archives are built with `clift` only: one archive per platform, `compiled tier: cranelift` (#780).
  The test runner takes the first
  `MAJOR.MINOR[.PATCH]` it finds; the prerelease tag is ignored for ordering.
  The version is available to the test runner packages as the `yavs_version` marker.

The test runner's capability answers for yavs (`SimulatorInterface` hooks):

| Hook | Answer | Why |
|---|---|---|
| `supports_vhdl_contexts` | True | 2008 |
| `supports_vhdl_package_generics` | True | M3 |
| `supports_vhdl_call_paths` | True | 2019 `std.env` call paths (`get_call_path`, `file_name`, `file_line`), implemented (#99) |
| `has_valid_exit_code` | True | see section 5 |
| `supports_vhpi` | True | M5 (`--vhpi`) |
| `supports_coverage` | False | `--cov` exists, the runner does not drive it yet (section 8) |
| `supports_gui_flag` | True | waves opened in the viewer, section 6 |

## 2. Standards

`--std=93` (also `1993`), `--std=2008` or `--std=2019`. Any other value (`87`, `02`, ...) is a
usage error (exit 2). The test runner raises an error itself for a project containing
VHDL-93/2002 files, so it never passes `93`; the value is for people and scripts that call yavs
directly (`docs/design/vhdl-93.md`).

- A project may mix 1993, 2008 and 2019 files. `analyze --std=STD` gives the
  standard of that one file (default 2008), and yavs records it per file in the
  library manifest (`1993`, `2008`, `2019`). The test runner passes each file's own standard.
- `run`/`elab` re-analyze every file with its recorded standard. Their `--std`
  is optional and the test runner does not pass it: the whole design uses ONE `std`
  and `ieee` revision, the newest standard recorded for any file in the
  design's closure (not the top level unit's standard), and that revision
  selects the run-time semantics of the whole design (e.g. 64-bit `integer`
  in 2019, plan rule 7; in a design of 1993 files alone `std_logic_vector` is a type of its own
  and the 1993 `std_logic_1164`, `numeric_std`, `std.textio` are used). `--std` on `run`/`elab`
  overrides it. A file using a
  declaration or construct that is not available under that revision fails
  early with `E1007`, naming the file and its recorded standard (exit 2),
  never a silent reinterpretation. A construct whose meaning changes under
  that revision is warning `W1008` (once per construct; `-Werror=W1008`
  escalates it). A 1993 file in a 2008 or 2019 design sees that design's packages, so 1993 code
  that overloads on both `std_logic_vector` and `std_ulogic_vector` fails with the usual
  homograph error there. Mixing is never refused: yavs elaborates a design of 1993 and 2008 (or 2019)
  files with the newest packages, where the reference simulators refuse to mix standards in one
  library or elaboration (user decision, 2026-10-23). See `docs/design/vhdl-93.md` section 3.3 and
  `docs/design/sem.md` section 0.1.

## 3. Libraries

- A library is a directory. `--work=NAME:PATH` names the library being analyzed
  into (or the library holding the top level, for `run`/`elab`).
  `--map=NAME:PATH` maps any other library. The test runner passes `--map` for every
  library in the project, including the work library: a `--map` identical to
  `--work` must be accepted.
- A library directory holds `yavs-lib.toml` (the index: the files in analysis order with their standard,
  content hash, size and modification time, and the units each defines), `yavs-lib.d/` (one record per source
  file with the unit names and dependencies, named by a digest of their text), `_YAVS_LIB` (the lock) and
  `yavs-cache/` (below). Only yavs reads and writes them: the test runner never parses them, and the layout
  has a `format` number in the index (3). A library written by a yavs with another number is refused with
  `L0004` naming the directory; delete it and analyze its files again (libraries are not converted).
- A missing or empty directory is a valid empty library. `analyze` creates the
  directory (and its parents) on first use. The test runner does not run an init command;
  it only creates the parent of the library directory. This holds for every library, the `--work`
  library of `run` and `elab` included (#838): they never create it, and a top level that is not
  in it is error `E0104` (exit 2), "top level `X` not found in library `work`", whose help says
  the library holds no design units and whether its directory does not exist or has no analyzed
  files. `E0104` is also the error for a wrong `TOP` in a library with units (the help suggests
  a similar name), and explains that `TOP` takes no library prefix (`work.top`). `L0031` remains
  the error for a unit that the design needs and the library lacks.
- `std` and `ieee` (and any other library yavs ships) are resolved by yavs
  itself, on demand from the std/ieee sources built into the `yavs` executable
  (nothing is installed next to it). `$YAVS_STDLIB`, if set,
  names a tree on disk (`<dir>/{std,ieee}/<rev>/*.vhdl`) used instead. A user `--map` or
  `--work` of `std` or `ieee` is an error (`E0102`), except under
  `yavs analyze --bootstrap`, which is how yavs's own `lib/std` and `lib/ieee` are
  analyzed (tools/corpus `yavs-lib-*`): the mapped `std`/`ieee` directories then
  replace the vendored tree for that analysis.
- Library names are case-insensitive VHDL identifiers; paths are used verbatim
  and may contain spaces and, on Windows, drive letters (`C:\x`): split
  `NAME:PATH` at the first colon only.
- Concurrency (plan rule 14): `run` and `elab` never write a manifest or lock a library
  for writing. The only thing they write into a library directory is the compilation cache
  entry of the work library (below, in `<work dir>/yavs-cache/`), best effort and by atomic
  rename, so any number of them may run at once against the same libraries. They never write
  analysed-unit entries (`yavs-cache/units/`); `analyze` does, by atomic rename as well.
  `analyze` must be safe when several processes analyze into the same library
  at the same time (two `run.py` invocations, or a future parallel compile in
  the test runner). yavs does this with automatic library locking like the reference simulator's: each library
  directory has a `_YAVS_LIB` lock file held exclusively by `analyze` while it
  updates the library (each record and the index are written to a temp file, then atomically
  renamed, the records first and the index last, so a reader sees the index it read and the records it
  names, whole); `run`/`elab` hold it shared only while reading the index and the records of the
  files in the closure. Read-only
  library directories never fail: shared lock via a read-only open, or no lock
  if the file cannot be opened at all. The test runner needs no locking of its own.
- Edited sources: `run`/`elab` find a file that changed since it was analyzed (`L0034`) by its content
  hash. When `analyze` recorded a file that had not been modified for two seconds, the index also
  holds its size and modification time (`size`, `mtime_ns`), and a file that still has both is not
  read and hashed again. So content of the same size written back under the recorded modification
  time goes unnoticed (as with build tools in general); re-analyze such a file. A file recorded without
  the two fields (one edited right before it was analyzed) is always hashed.

### Compilation cache

`yavs elab` and `yavs run` keep what analysis of the design's closure produced in the work
library's directory, `<dir>/yavs-cache/sem-<key>.img` (user decision 2026-10-19, `docs/decisions.md`
"Compilation cache"). The next `elab`/`run` of the same closure reads it instead of parsing and
analyzing the sources again; the output, the diagnostics (warnings included) and the exit status are
identical to a run without it, at any `--threads`.

- The key is a hash of the yavs build, `--std`, `--relaxed`, `--psl`, `--encoding`, `-D` and every file of the
  closure (library, path, recorded content hash, standard). Editing a file (or any file it depends
  on) is found by the content hash check of the closure (`L0034`, re-analyze it); an entry also records
  the bytes of the built-in `std` and `ieee` files it was made from and is used only if they are
  unchanged. Entries of another
  yavs build, another format version or a damaged file are ignored, never an error. An error-free
  analysis is stored; a design with errors never is.
- Entries are written to a temporary file and renamed into place: concurrent runs need no lock and
  never see a partial entry. The newest 96 entries per library directory are kept. A library
  directory that cannot be written (read-only mount, no permission) is simply not cached into;
  reading from it works as before.
- `--no-cache` (`run` and `elab`) neither reads nor writes the cache. A custom `$YAVS_STDLIB` tree
  disables it. `yavs cache clean --work=NAME:PATH [--map=NAME:PATH ...]` removes the entries of those
  library directories and, with them, the shared std/ieee store (see below; `--std-cache=DIR|off` picks or skips it), and prints what it removed per directory (`E0123` if one cannot be removed). Deleting
  the `yavs-cache` directory by hand is equally safe.
- The first run of a closure only leaves a note (`<kind>-<key>.seen`, an empty file) and makes no entry
  of its own; the second run makes the analysis image and the lowered packages. A closure that starts
  with the files of a closure analyzed before (same options, same files with the same contents)
  continues from a prefix entry (`pre-<key>.img`) kept in the cache of the library of the prefix's last
  file (made by the second different closure that starts with the prefix, the first only leaves a note);
  the output is the same as from source. A closure under a prefix whose library is lowered and kept
  (kinds `lib`, `libopt`, `libinit` beside the prefix entry) makes no entries of its own at any run.
- The lowered form of the packages with run-time objects (signals, shared variables, constants
  that are not static) is kept beside it, `<dir>/yavs-cache/low-<key>.img`, when the design is
  elaborated without instantiating anything at elaboration time and the lowering is neither tiny
  nor huge; the same rules apply (identical results, damaged or foreign entries ignored, atomic
  writes, `--no-cache`, `yavs cache clean`). It counts toward the 96 entries.
- The lowered design (the architectures that own processes and every subprogram they reach, with
  the optimised copies of the functions earlier runs called) is kept beside it,
  `<dir>/yavs-cache/ir-<key>.img`, under the same rules; a run that called functions the entry
  lacks writes the entry again when it ends. The IR verifier runs on every function when the
  program is finished in debug builds and with `YAVS_VERIFY_IR` set; otherwise a function is
  verified when first called, unless the program was stored (verified when stored).
- The state of those packages after their initialisation (package data, shared variables, the shared
  heap with protected objects) is kept as `<dir>/yavs-cache/init-<key>.img` under the same
  conditions and for the same time resolution, and a run restores it instead of running the
  initialisers. The reports the initialisers made (`** Note: (init): text`) are printed again, in the
  same order and counted for the exit status. It is not kept when an initialiser opened or used a
  file, read the environment or a clock, or called foreign code: those run every time. Results,
  messages and waveforms are identical with and without it. It counts toward the 96 entries.
- A closure's first run also uses what the files it starts with give once for all closures of that
  prefix, kept in the cache of the library of the prefix's last file: the lowered library
  (`lib-<key>.img`: the packages and every subprogram they declare that does not depend on the design),
  the state of its first packages after initialisation (`libinit-<key>.img`) and the optimised copies
  of its functions (`libopt-<key>.img`). The library is made when four different closures have asked for the prefix, by
  one run at a time (`<kind>-<key>.making` while it does), and only when no package body follows the
  prefix, nothing was instantiated at elaboration time and the run has no coverage counters and is not
  `--tier=llvm`; otherwise the run lowers as before. Results, messages and waveforms are identical with
  and without it. A library directory keeps at most 96 entries and 512 MiB of them.
- With `--tier=llvm` or `--tier=auto`, the machine code of every module the compiled tier makes is
  kept in `<dir>/yavs-cache/obj/obj-<key>.img` of the work library (#505); a later run that makes a
  module with the same key (the same generated code, options, target and yavs build) links it instead
  of compiling it. Same rules: identical results, damaged or foreign entries ignored and compiled
  again, atomic writes, nothing stored into a directory that cannot be written, `--no-cache`, `yavs
  cache clean`. The directory keeps at most 4096 objects and 256 MiB, apart from the 96 entries.
  With `--tier=clift`, `--tier=clift-auto` or `--tier=auto3` the machine code of every function
  Cranelift translates goes to the same directory as `clif-<key>.img` (the function's translated
  body, the target and the yavs build; #481), under the same rules and limits; `--tier=auto3` keeps
  its LLVM modules there too.
- **Analysed units** (#537, #539, user decision 2026-10-22, `docs/decisions.md` "Analysed-unit
  cache"): `yavs analyze` keeps the analysis of every design unit it analyses from source in
  `<dir>/yavs-cache/units/unit-<key>.img` of the unit's library (the built-in `std` and `ieee` units in the
  store shared by all libraries, below), and a later `analyze`, `elab` or `run` that needs the unit reads it from there instead of
  parsing and analysing its file again: the dependencies of a file analysed one file per call, a
  file analysed again unchanged, the closure of a first `elab` or `run`. An entry is used only if the
  unit's file has the content (and standard) it was made from, the units it depends on are the ones
  it was made with (an edit anywhere upstream invalidates everything below it), the yavs build and the
  options (`--std` of the design, `--psl`, `--encoding`, `-D`, and `--relaxed` where the unit's analysis depends on it) are the same and the
  libraries answer what they answered then; anything else, a damaged entry included, is a miss and the
  unit is analysed from source. Diagnostics, warnings included, and results are identical either way. `yavs analyze` shows warnings and notes only for the files named in the call (a unit read from the library was reported when its file was analysed, #584; errors are always shown), so a file that uses a unit with a warning does not repeat it, and `-Werror` counts the named files' warnings only.
  Only `analyze` writes entries (under its library session; the units of other libraries and of
  `std`/`ieee` by atomic rename, no lock); `elab` and `run` only read them. A library directory that
  cannot be written is read and never fails. `analyze --no-cache` neither reads nor writes them; `yavs
  cache clean` removes them. The directory keeps at most 16384 entries and 512 MiB, apart from the 96
  entries.
- **Shared std/ieee units** (#564, user decision 2026-10-26, `docs/decisions.md` "Shared std/ieee units"):
  the analysed units of the built-in `std` and `ieee` are kept once per user in
  `$YAVS_STD_CACHE/yavs-cache/units/`, default `yavs/std` in the user's cache directory (`$XDG_CACHE_HOME`
  or `~/.cache`; `%LOCALAPPDATA%` on Windows), not in each library. A fresh library therefore reads them
  instead of analysing them. The rules of the unit cache hold (identical results, damaged or half-written
  entries are misses, atomic creation, no lock). `--std-cache=DIR|off` (`analyze`, `elab`, `run`, `cache clean`) names the
  directory instead of the environment and wins over it. `off` (flag or `YAVS_STD_CACHE=off`) keeps them in each library's own
  directory. `yavs analyze` checks at start that the directory can be written: `E0126` names the directory,
  the cause and the fix; an empty value (flag or environment), or no cache directory to derive one from, is `E0126` for `analyze`,
  `elab`, `run` and `cache clean` (not with `--no-cache`). A user's own `ieee` (`analyze --bootstrap`) is a mapped library and
  is never shared. Deleting the directory is safe.

## 4. `yavs analyze`

```
yavs analyze --std=FILE_STD --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
             [--bootstrap] [yavs.global_flags] [yavs.a_flags] FILE...
```

- One or more files per call, in dependency order (the test runner `--batch-compile` /
  `--parallel-compile`: directly as several arguments, or as one `@filelist`
  response file with a file per line, section 0). All files get the same options
  and `--std`. They are analyzed in the given order by one analysis session: each
  file is parsed once and sees the units of the files before it, even before the
  index is read again. The work library's exclusive lock is held for the
  whole call, and each successful file's record is renamed into place as soon as it is done; the index is
  written once, at the end of the call and after a failure (#747: one write for the batch, not one per
  file; a reader waits for the lock, so it sees the library as it was before the call or after it, and a
  call killed half way records none of its files, which the next `analyze` redoes from the analysed-unit
  entries it made). Nothing is fsynced, so a power loss may lose the last call's files, which the next
  `analyze` records again. Concurrent calls into the same library serialize per call and never lose each
  other's files. A call reads the index and writes the records of its own files and the index once, not
  the whole library file by file.
- Integrations pass the consecutive files of one library and standard in one call by default (user decision
  2026-10-25, #745: fewer process starts and one loaded index; cold analysis of the verification libraries
  costs 26 to 50 percent fewer user instructions and about 85 percent fewer kernel instructions); where each
  file's own result is needed, a failing call is repeated file by file.
- Exit 0: every file was analyzed and recorded in the work library. Warnings go
  to stderr/stdout and do not change the exit code (unless `-Werror`).
- Exit 1: a file has errors. The call stops at the first failing file (fail
  early); files before it stay recorded, the failing one and the rest are not
  analyzed. With more than one file, stderr names the failing file and lists the
  files analyzed and recorded before it.
- `--bootstrap` allows `std`/`ieee` as `--work`/`--map` (section 3).
- Exit 2: usage error (bad option, unknown standard, unmappable library).
- Exit 3: internal compiler error.
- The test runner shows the combined stdout/stderr to the user on failure, so the default
  human format is the right one; the test runner does not request
  `--message-format=json` (section 0).
- Options from plan rule 13 that users will put in `yavs.a_flags`:
  `--relaxed` and the `-W` forms below. They also work for `elab` and `run`.
  - `--psl` (also `yavs parse`): read `-- psl` comments as PSL pragmas, as the reference
    simulators do only when asked (native VHDL-2008 PSL statements are always read). Pass it to
    `analyze`, `elab` and `run` alike, as the sources are read again at elaboration.
  - `-D NAME=VALUE` (`--define`, also `yavs parse`; repeatable): define a conditional analysis identifier
    (`` `if NAME = "VALUE" ``). Pass it to `analyze`, `elab` and `run` alike, like `--psl`: the library does
    not record it, `elab` and `run` read the sources again, and an identifier they do not know is `E0007`
    (#837). The test runner's global flags reach all three.
  - `-Werror`: every warning is an error (`analyze`: exit 1 after the batch; `run` and `elab`: exit 2,
    as any error before the simulation starts).
  - `-Werror=CODE`: that warning is an error. `-Wno-CODE`: that warning is not
    printed. `-WCODE`: that warning stays a warning even under `-Werror`.
  - For one code the last option wins; `CODE` must be a registered warning code
    (`W0001`, `W1008`, ...), else `E0103` and exit 2.
- An encrypted source (IEEE 1735 `` `protect begin_protected `` envelope, #775) is one error
  `E0012` naming the file and the line that opens the envelope, with the fix; yavs never decrypts.
- `yavs --explain CODE` prints the long text of a diagnostic code on stdout
  (exit 2 for an unknown code).
- A panic in any command is an internal compiler error: `E9999` on stderr and
  exit 3.

## 5. `yavs run` and `yavs elab`

```
yavs run  [--std=STD] --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
          [--ieee-warnings=off] [yavs.global_flags] [yavs.elab_flags]
          [yavs.sim_flags] --exit-severity=LEVEL [--wave=FILE.fst]
          [--cov[=FILE]] TOP [-gNAME=VALUE ...] [+NAME[=VALUE] ...]
yavs elab [--std=STD] --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
          [yavs.global_flags] [yavs.elab_flags] TOP [-gNAME=VALUE ...]
```

The test runner passes everything after `run`/`elab` in a response file (Conventions).

- `run` elaborates and simulates in one process (no elaboration artefact is
  saved; only the analysis is cached, see "Compilation cache" in section 3). `elab` elaborates only, for the test runner's
  `--elaborate` mode, and has the same exit codes as `run` minus the
  simulation ones.
- `TOP` is `ENTITY`, `ENTITY.ARCH` or `CONFIGURATION`, looked up in the work
  library. The test runner always passes `ENTITY.ARCH` or a configuration name. No
  library prefix.
- Generics: `-gNAME=VALUE`, one argv element each, split at the first `=`.
  `NAME` is case-insensitive. `VALUE` is parsed according to the generic's type
  with the normal value parser (plan rule 4); for a `string` generic it is the
  raw text, taken verbatim, without VHDL quotes. It can be long (the test runner's
  `runner_cfg` is several hundred bytes and grows with the number of test
  cases) and contains spaces, `:`, `,`, `/`, `\` and `.`: yavs must not split or
  unescape it. A `-g` for a generic the top level does not have is a warning,
  not an error. Also accept `-gNAME=VALUE` after `TOP` (the test runner puts them there).
- Plusargs: `run` accepts arguments `+NAME` and `+NAME=VALUE` after `TOP` (anywhere among the
  `-g` options), one argv element each, taken verbatim. They are the tool's arguments for VHPI
  plugins: `vhpi_get(vhpiArgcP, tool)` is `1 +` their number and the `vhpiArgvs` iterator of the
  tool object yields the program name `yavs` first, then the plusargs in order
  (`vhpi_get_str(vhpiStrValP, arg)`). Nothing else uses them. Any other extra argument is a usage
  error (exit 2). Python testbench frameworks append their plusargs this way.
- `--exit-severity=LEVEL` (`note|warning|error|failure`): an assertion or
  report at or above LEVEL stops the simulation at the end of the current delta
  and gives exit 1. Without the option the level is `failure` (user decision 2026-10-18, as
  the other major simulators): an `error` report is printed and the simulation continues, and
  the run still exits 1 at the end because an `error` (or higher) was reported, unless
  `ClearVhdlAssert` (2019 `std.env`) cleared the assert counts since: the status follows the
  error and failure counts at the end of the run. With the option given, even as
  `--exit-severity=failure`, the level is the whole threshold (user decision 2026-10-21, #498,
  as the other major simulators): only reports at or above it affect the exit status, so a run
  that prints `error` reports and ends normally exits 0. yavs's own
  runtime errors (bound check, overflow, null access, delta limit, internal) stop
  at once whatever the level. The test runner passes its `vhdl_assert_stop_level` sim option.
- `--ieee-warnings=on|off` (default `on`; any other value is a usage error, exit 2): `off` suppresses the
  warnings raised inside `ieee` packages (the test runner `disable_ieee_warnings`), for example
  `NUMERIC_STD.TO_INTEGER: metavalue detected`. Only reports of severity `warning` made by a statement of
  the `ieee` library's own sources are dropped (the native subprograms and the VHDL bodies alike,
  also when the optimiser inlined the body); they are neither printed nor counted, so a user's own
  `report ... severity warning` and every other severity stay (#836).
- `--wave=FILE.fst`: write an FST of the whole design. Hierarchy follows the
  elaborated design (instances, generate scopes, records and arrays as scopes);
  std_logic family and bit are written as characters, other enumerations with an
  enum table, integers, reals and time as numbers (an integer with its subtype's name and
  the width its range needs; a generate alternative with a label is scope `label.alt`). Several deltas at one time step
  give one value change (the final value). The file is the same at any
  `--threads`. Names that are the same value (a port collapsed into its actual, at any depth) share one value chain in the file (FST aliases). Signals and ports are recorded; process variables only with `--wave-variables`. Flushed when a block fills (or every `YAVS_WAVE_FLUSH` interval, off by default), on normal end, on stop, and on SIGINT: the
  first Ctrl-C ends the run between cycles, finishes the file, prints
  `simulation interrupted` and exits 1; a second Ctrl-C exits at once (the file is
  readable up to its last flush). An unwritable path is `E0801` (exit 2).
- `--wave-include=GLOB`, `--wave-exclude=GLOB` (repeatable): record only paths
  (`top.u1.sig`, case-insensitive, `*` one level, `**` across, `?`) that match /
  skip those that match. `--wave-max-array=N` (default 65536): arrays and vectors
  with more elements are left out with warning `W0802`. Signals left out by either
  are not read while simulating. The test runner does not
  pass these; use `yavs.sim_flags`.
- `--wave-variables` (needs `--wave`, default off): also record process variables (scope =
  the process label, path `top.proc.var`) and non-protected shared variables (in their region,
  `top.u1.v`, or package, `lib.pkg.v`), sampled at the end of each time step; same file at any
  `--threads`. Records and arrays are scopes as for signals, integers have the width of their
  elaborated range, and the globs and `--wave-max-array` apply. Access, file and protected
  variables are not recorded; a variable whose bounds
  are known only at run time is left out with `W0802`.
- `--resolution=fs|ps|ns|us|ms|sec` (`run`, default `fs`): the time resolution limit, a run-time
  setting (plan rule 7). It is the value of `std.env.resolution_limit`, the unit of TIME
  values in `std.reflection` mirrors (decisions.md "Reflection: TIME values in mirrors") and the
  unit delays are truncated to (the kernel). Any other value is a usage error (exit 2). Not
  applied to `yavs elab` (evaluation during elaboration uses `fs`). LRM 5.2.4.2: a non-zero
  TIME literal (or bare unit name) with a unit below the limit anywhere in the elaborated
  design, packages and dead code included, is error `E2120` before time 0 (exit 2; the first 10
  locations are listed); `0 fs` and values such as `0.4 ns` under `ns` are legal. The built-in
  std and ieee sources are exempt. A design that ran with a coarse limit before can now fail.
- `--stop-time=TIME` (e.g. `10ms` or `10 ns`; a bare number is `E0105`), `--threads=N` (at least 1; 0 is a usage error, exit 2, #839), `--shuffle`, `--load=LIB`,
  `--vhpi=LIB[:ENTRY]`, `--dap=PORT`: not passed by the test runner, used through
  `yavs.sim_flags` or simulator hooks (a test runner package can add `--load`).
- Stuck and hung simulations (#582), `run` only, all as diagnostics (code, labels, notes, help; JSON
  with `--message-format=json`; `yavs --explain CODE`):
  - `E3002` (delta cycle limit) names the processes and signals of the loop found from the drivers
    and waits at the time of the error (two processes waking each other, a process waiting only on a
    signal it assigns, a combinational loop through concurrent statements, `wait for 0 ns;` in a
    loop), labels their places and says how to raise the limit. `--max-deltas=N` (default 10000, at
    least 1) sets the limit. Nothing is recorded during the run.
  - `W3101` is the hang report: time and delta, how many processes wait for a timeout, for signals
    that can still change, or can never run again; for each of those processes (at most MAX per
    part) what it waits for, and for each signal that nothing can change its value, its last event
    and the processes that drive it (`it has no driver`); a clock (a signal an edge-triggered
    process waits for) that had an event and cannot toggle any more is named. Printed by
    `--hang-report[=MAX]` (default 30) when the run ends at `--stop-time` without `stop` or `finish`,
    or runs out of events while a process still waits for a signal (a run whose processes all
    ended or are at `wait;` is a clean end and prints nothing); the end of a run is silent without
    the option, because a stop time is the normal end of many testbenches. On Unix, SIGUSR1 prints
    the same report (headed `simulation status at TIME`) of a running simulation, which then
    continues; the signal is handled between two delta cycles, so a process that never suspends
    (`W3102`) cannot be reported. SIGUSR1 is not handled on Windows. The report goes to stderr; a
    warning does not change the exit status (`-Werror` and `-Wno-W3101` apply).
  - `W3102`: a process whose body has no wait statement, no sensitivity list and no call of a
    procedure that waits, printed before time 0. It never gives control back, so time cannot
    advance (only `stop` or `finish` ends the run).
  - `--shuffle[=SEED]` (seed 1 without a value) prints ``note: process order is shuffled with seed N;
    replay it with `--shuffle=N` `` on stderr at the start, and the runtime diagnostics of a shuffled
    run repeat the seed. That is the only run-time randomness; there is no time-based seed.
- `--no-collapse-ports` (`run` and `elab`): keep every `in` port its own signal. By default the
  formal of an `in` port whose actual is a signal name (a whole signal, record elements, an
  element or a slice), with no conversion and no force or release of the port, is an alias of its actual: no separate signal
  and no copy per delta. Output, waves and `'path_name` are the same either way; a forced port,
  and every port under `--dap` or `--vhpi` (a debugger or plugin may force or deposit any port),
  stay apart. For comparing against the uncollapsed design.
- `--time-report`: when the simulation ends (any outcome after elaboration) print the wall and
  CPU seconds of each phase to standard error, one line per phase and a total:
  `time-report: PHASE wall=S cpu=S`, with PHASE one of `analyze` (library loading, parsing and
  analysis of the design closure), `elaborate` (elaboration and lowering), `init` (package and
  instance initialisation, plugin and waveform setup) and `simulate`, then `time-report: gc collections=N` (heap collections of the whole run) and `total`. CPU is the
  whole process (all threads) in 10 ms steps and `n/a` where the platform does not tell (read
  from `/proc` on Linux). Off by default and then free. Standard output and exit status do not
  change; a run that fails before the simulation starts prints nothing.
- `--profile[=FILE]` (`run` only, `=` form required): a sampling profiler. A thread samples the
  run 1000 times a second and, when the simulation ends (any outcome after elaboration, and
  Ctrl-C or SIGTERM: the first signal stops the run between cycles, the profile of the time so
  far is printed and written, and the exit status is 130 as without `--profile`; a second
  signal exits at once without a profile; Unix, on Windows Ctrl-C is handled the same way by
  the console handler but is untested), prints five blocks to standard error, each line starting
  `profile: `: a summary (samples, seconds, share in processes and in the kernel between them),
  `processes` (instance path of the process, source file and line of its first statement),
  `instances` (the processes of each instance path together), `subprograms` (name, file and line of
  the first statement, samples in the interpreter and in compiled code) and `lines`
  (file:line and the subprogram that owns the statement, interpreted code only); the 20
  busiest rows of each. With FILE the same tables, all rows, are also written there as JSON
  (`hz`, `seconds`, `samples`, `kernel_samples`, `processes[]` with `path`, `file`, `line`,
  `samples` and `subprograms[]`, `instances[]`, `subprograms[]` with `interpreted_samples` and
  `compiled_samples`, `lines[]` with `file`, `line`, `subprogram`, `samples`); a file that
  cannot be written is `E0124` and the exit status at least 1. Standard output and the exit
  status are those of a run without the option. A sample counts every simulation thread that
  runs a process, so with `--threads N` the shares can add up to more than 100 percent. The
  interpreter publishes the subprogram and statement it is in every 128 loop iterations or
  calls (the subprogram exactly at every call with `--tier=llvm` or `auto`), so short
  activations may show none, a statement is where the interpreter last looked, and compiled
  code has no `lines` rows. A small subprogram the optimiser inlined counts as its caller in
  `subprograms`, but its statements are `lines` of their own function. The sampler is a thread
  that sleeps between samples: no signal or timer is used. Time in plugin callbacks (`--vhpi`) is
  in the kernel share. With `--dap` it is `E0125` (exit 2). Off by default and
  then free: no process is wrapped and no thread exists.
- `--load=LIB` (repeatable): load a shared library for VHPIDIRECT foreign
  subprograms. A bare name goes to the OS library search path, a path must exist.
  Every foreign subprogram the design calls is bound before the simulation starts:
  its symbol is looked up in the `--load` libraries in order, then in the
  executable. A library that cannot be loaded is `E0106` and a signature the
  trampolines cannot call is `E0107`; both exit 2. A symbol defined nowhere is
  warning `W0108` before time 0 (one per subprogram) and `E0107` only when the
  subprogram is called (a runtime error, exit 1). No VHPI
  registration runs for `--load` libraries (that is `--vhpi`).
- `--vhpi=LIB[:ENTRY]` (repeatable, in order): load a VHPI plugin after elaboration and before
  initialisation. Without `ENTRY` the library's `vhpi_startup_routines` run, with it that
  function. A plugin that cannot be loaded is `E0901`, a missing entry point `E0902`; both exit 2
  before time 0. `vhpiCbStartOfSimulation` and `vhpiCbStartOfInitialization` fire before
  initialisation, `vhpiCbEndOfSimulation` once when the run ends (also after a stop or finish).
  `vhpi_control(vhpiStop | vhpiFinish)` ends the run between cycles with status 0 (prints the
  same `STOP`/`FINISH called` note as `std.env`); other commands are `W0904`. Simulation
  callbacks (#272): `vhpiCbEndOfInitialization` after every process ran once,
  `vhpiCb[Rep]NextTimeStep`, `vhpiCb[Rep]StartOfNextCycle`, `vhpiCb[Rep]StartOfProcesses`, `vhpiCb[Rep]EndOfProcesses` (#317), `vhpiCb[Rep]LastKnownDeltaCycle`
  (the read-write phase), `vhpiCb[Rep]StartOfPostponed`, `vhpiCb[Rep]EndOfTimeStep` (the read-only phase, after postponed
  processes) and `vhpiCb[Rep]AfterDelay`. VHPI times (`vhpi_get_time`, callback delays, the
  `time` field passed to callbacks) count in units of `--resolution`, which is what
  `vhpi_get_phys(vhpiResolutionLimitP)` returns; `vhpi_get_time` gives the delta count as `cycles`. Plugins see
  the elaborated design hierarchy (#269: regions, declarations, record fields and array
  elements, lookup by name and index; names as in `crates/yavs-vhpi/AGENTS.md`) and read
  values with `vhpi_get_value` (#270, formats as in `crates/yavs-vhpi/AGENTS.md`);
  value-change callbacks and `vhpi_put_value` need #271.
- `--dap=PORT|stdio`: run under a Debug Adapter Protocol client instead of printing to stdout.
  With a port, yavs listens on `127.0.0.1:PORT` (`0` picks a free one) for one client, and its
  first line on stderr is exactly `yavs: DAP listening on 127.0.0.1:<port>`, the second
  `yavs: waiting for a debugger on port <port>`, and nothing runs until the client attaches; with `stdio` stdout
  carries only DAP messages (file descriptor 1 is moved to stderr, so a plugin's or foreign
  library's prints appear there). Reports and `OUTPUT` text become `output` events, a runtime error
  an `output` event of category `stderr`. Processes run on one thread (`--threads` above 1 is
  warning `W0109`), so stops are deterministic. The exit status is the run's (below), sent as
  the `exited` event before `terminated`; a disconnect before the end stops the run (exit 1).
  `YAVS_COUNTERS` prints the run's work counters on stderr when the session ends, as without `--dap`.
  `--wave` and `--vhpi` work with `--dap` (the waveform file is flushed at every stop and finished before `exited`; a
  plugin's own prints go to stdout, so use a port with `--vhpi`); `--cov` with `--dap` is
  `E9999` not implemented (exit 2). A port that cannot be opened is `E0701` (exit 2). Custom requests, served while stopped: `yavs/runFor` and
  `yavs/runUntil` (argument `time`, a TIME literal such as `"10 ns"` or `"10ns"`, relative to the
  current time or absolute), `yavs/stepDelta` and `yavs/stepTimestep` (no arguments); each
  answers, sends `continued`, and the next `stopped` event's `description` gives the time and
  delta. Signal breakpoints are DAP data breakpoints (`supportsDataBreakpoints`; `dataId` is
  the signal's lower-case path `:top:u1:s`) and the custom request `yavs/setSignalBreakpoints`
  (`breakpoints: [{signal, value?}]`, `frameId` for names seen from a process; it replaces its
  own set): a stop after the cycle in which the signal changed (to `value`), reason `data
  breakpoint`, `text` `PATH = VALUE`. `yavs/force` (`signal`, `value`, `mode` `in` or `out`,
  default `in`), `yavs/deposit` (`signal`, `value`) and `yavs/release` (`signal`, `mode`) act on
  a signal or a part of one (`signal` a name seen from `frameId` or the stopped process, or a
  path, optionally followed by fields, integer indices and a final slice in the signal's
  direction: `r.a`, `v(3)`, `:top:m(2).f`, `d(7 downto 4)`; the value is of the part's type) in
  the next cycle, and only on those elements; they answer with the target and the value as
  text. `setVariable`
  (`supportsSetVariable`) deposits a signal (a field or element only that part) and writes a local at once; generics are refused.
  Values are VHDL literals of the target's type: scalars as `T'VALUE` reads them (physical values
  in any unit), string and bit string literals, aggregates with positional and named elements,
  ranges and `others`; a value that does not parse is a failed response naming the problem.
  `condition` on line and data breakpoints (`supportsConditionalBreakpoints`) is a VHDL boolean
  expression over the stopped frame's locals and the signals visible from the process (or the
  watched signal's region) or by path, with predefined operators; one that does not parse or
  uses something unsupported is `verified: false` with a `message`, an error while evaluating a
  stop with reason `exception`. `hitCondition` counts the hits where the condition held.
  Composite variables have a `variablesReference` (records with `namedVariables`, arrays with
  `indexedVariables`, children named by field or index, paged by `start` and `count`).
  The scope `Hierarchy` and the custom request `yavs/hierarchy` (answered at any time;
  `regions: [{name, path, kind, regions, processes, signals: [{name, path, type}]}]`) give the
  elaborated hierarchy. Live waves (answered at any time): `yavs/waveSubscribe` (`include`,
  `exclude`: globs as `--wave-include`/`--wave-exclude`; `maxArrayElems` as `--wave-max-array`;
  `variables` as `--wave-variables`; `flushMs`, default 100) replaces the subscription and
  answers with the recorded hierarchy as `--wave` declares it (`scopes: [{scope, kind,
  component, children}]`, signals `{name, id, type, encoding: {layout, width}, class, direction}`, `skipped`);
  `yavs/waveUnsubscribe` ends it. Event `yavs/waves {from, to, times: [fs], changes: [[time
  index, id, value]]}` carries the final value per completed time step of every recorded value
  that changed (value = base64 of the recorder's value bytes, decoded by the signal's `encoding.layout`:
  `u8-per-element` (a bit/`std_logic` vector, one enumeration position per element, `width`
  elements, left to right), `u32le` (enumeration position, index into `type.literals`), `i64le`
  (integer, TIME in fs), `f64le` (real); times as integers in fs),
  sent at the end of a time step once `flushMs` have passed or enough text is pending, and
  always before `stopped`, `exited` and the `yavs/waveUnsubscribe` response. The simulation
  never waits for the client: when too many batches are queued the newest is merged into the
  last (`merged: true`, only the last change of each value kept). A failure of the live waves
  is `E0702` (the run fails). Supported requests and the id scheme: `crates/yavs-dap/AGENTS.md`.
- `--tier=interp|llvm|auto` (`run`; default `auto` in a build with the `llvm` or the `clift` feature unless `--dap` is
  given, else `interp`, quietly: no error, no warning; user, 2026-10-22, #510): `llvm` compiles every function of the
  design with the LLVM tier before time 0 and runs that code; `auto` starts in the interpreter
  and compiles each function that becomes hot (once the instructions it ran in the interpreter
  would have paid for its compilation) at the cheapest optimisation level on one background
  thread, swapping the code in while the simulation runs, which never waits for it.
  The output is identical to `interp` whatever the tier and whenever compilation finishes (the
  interpreter is the reference). At the end of an `auto` run, functions still queued are
  dropped; if one is still being compiled, yavs flushes its output and exits at once without
  running exit handlers, unless `--load` or `--vhpi` libraries are loaded: then it waits for
  that compilation so their exit handlers run. Elaboration-time
  code (package and instance initialisation) runs interpreted either way, and so does a run
  under `--dap`. Under `--dap` an explicit `--tier` is checked like without it (a tier the build lacks is `E0120`, exit 2;
  #835), but nothing is compiled (`--verbose` prints no tier notes): the debugger needs the interpreter's frame
  slots. `--tier-max-size=N` (default 50000): with `llvm` or `auto`, a function of more
  than N IR instructions stays interpreted (user, 2026-10-19, #126). `--tier-heat=BASE,PER_INSTR,PERIOD`
  (hidden, for tests and tuning): with `auto`, a function is hot once a sample every PERIOD
  microseconds has seen the interpreter in it for BASE microseconds plus PER_INSTR nanoseconds
  per IR instruction of it (PERIOD `0`, as in `1,0,0`: from its first call or jump). `llvm` needs a yavs
  built with the `llvm` cargo feature (LLVM 21), `auto` one built with `llvm` or `clift`: otherwise asking for them
  explicitly is error `E0120` (exit 2). `--help` for `--tier` and `--explain E0120` name the tiers the executable
  has (#845, #850); a
  design the tier cannot compile (or a compile thread that cannot start) is `E0121` (exit 3, a
  bug in yavs); an LLVM library that cannot be loaded (Linux: `lib/` next to `yavs`, else the
  system's) is `E0127` for `--tier=llvm` (exit 2), and a warning for `--tier=auto`, whose hot code stays
  interpreted. `clift` and `clift-auto` are `llvm` and `auto` with the Cranelift code generator; they need a yavs built with
  the `clift` cargo feature (no LLVM needed), otherwise `E0120`, and give the same output. In a build with `clift` and without `llvm`, `--tier=auto` is `clift-auto`, and so is the default tier of such a build (`interp` under `--dap`). `auto3` is `auto` with Cranelift as its first level: a function that becomes warm is translated by Cranelift, one that stays hot is compiled again by LLVM at O2 (a yavs built with both features, otherwise `E0120`); the output is the same. `--verbose` prints, on stderr, a note per function left to the
  interpreter and how many instructions were compiled (`llvm`), or a note per hot function as
  the compile thread takes it (`auto`, so their order and number depend on timing). The test
  runner passes neither; use `yavs.sim_flags`.
- Threads: yavs defaults to `--threads=1`. The test runner already runs `-p N`
  simulations in parallel and passes a thread count only when the user sets one: the sim
  option `yavs.max_threads` (or the runner argument `--yavs-max-threads N` for every test)
  becomes `--threads=N` before the raw flags, so `--threads` in `yavs.sim_flags` still wins; none
  under `--dap` or for `elab`. With the runner's `--yavs-jobserver` and `-p N` above 1, every
  `yavs run` gets a private GNU make jobserver in `MAKEFLAGS=-jN --jobserver-auth=fifo:PATH` (a
  named pipe, POSIX only; any outer `MAKEFLAGS` is replaced): the runner lends the CPU slots no
  test uses as tokens, at most `N - 1` per test and `yavs.max_threads - 1` when set, and takes
  back the tokens left unread. yavs takes and gives back tokens as the token pool item below
  says; tokens still held when it exits are a warning of the runner
  (python/vunit-yavs/README.md "Jobserver").
- Token pool (#801): with `--threads=N` above 1 (and not `--dap`), yavs reads `MAKEFLAGS` at
  the start of `run`. The last `--jobserver-auth=VALUE` (or `--jobserver-fds=VALUE`) names the
  pool: `R,W` (inherited pipe descriptors; Linux) or `fifo:PATH` (a named pipe; Unix). The
  simulation owns one implicit token and takes at most N - 1 more, without blocking, only
  when a group of processes goes to the threads; with none it runs that group on one thread.
  It writes back the bytes it read: after 64 groups in a row on one thread, at the end of the
  run (also after an error), and on the first SIGINT, SIGTERM or SIGHUP (a run that would have
  died of the signal then exits with status 130). A pool that cannot be used is error
  `E0128` at the start of `run`, before analysis (exit 2): the note gives the cause, the help
  the fix (a parent that does not share its pool removes `--jobserver-auth` from the child's
  `MAKEFLAGS`). Without a pool, or with
  `--threads=1`, nothing changes. Output and exit status do not depend on the tokens. The
  producer fills the pool with the bytes it wants (any value; they are returned unchanged).
  It must keep its own handle of the pipe open (a named pipe: opened for reading and writing)
  for as long as the simulation runs: a pipe's contents are lost when its last handle closes,
  so tokens written back at the end would be lost otherwise. `MAKEFLAGS` is split at
  whitespace, so the path of `fifo:PATH` must not contain any (E0128 otherwise).
- Garbage collection: `--gc-threshold=SIZE` (bytes, or a `k`, `m`, `g` suffix; default
  `1m`) sets how much access-value memory is allocated before the heap is collected. The
  test runner never passes it; a user who wants fewer collections opts in with
  `yavs.sim_flags`, e.g. `["--gc-threshold=16m"]`. The output does not depend on it.
- Exit codes:
  - 0: simulation ended without failure: event starvation, `std.env.finish`
    or `std.env.stop` with no status or status 0, or `--stop-time` reached.
  - 1: simulation failure: an assertion/report at or above `--exit-severity`
    (without the option: of severity `error` or `failure` at any point),
    `std.env.stop(N)`/`finish(N)` with N /= 0 (the status itself may be
    returned if it fits in 1..125; the test runner only tests zero/non-zero), a runtime
    error (bound check, null access, delta-cycle limit).
  - 2: usage, analysis-on-demand or elaboration error. A runtime error while package or
    instance data is initialised (before time 0) is an elaboration error: exit 2, reported
    like any runtime error below.
  - 3: internal error.
- The test runner itself writes the `vunit_results` file (`test_start:NAME`
  lines, then `test_suite_done`) into the `output path` from `runner_cfg`, via
  textio. yavs needs no test-runner knowledge beyond correct textio and `std.env`.
- Standard output: `report`/`assert` messages go to stdout in the human format,
  one message per line group. On a terminal each message is written at once. On a pipe or
  file, without a plugin (`--vhpi`), a `--load` library or a foreign subprogram, messages are
  collected and written in blocks: at the latest 100 ms after they were produced, before
  anything yavs writes to stderr, at the end of the run, and when Ctrl-C or SIGTERM ends it (exit status 130; SIGKILL cannot be caught and loses at most the last 100 ms or 64 KiB), so that the test runner's live
  output and timeouts work and the order of the two streams is the order of the events.
  A process that runs serially (every process at `--threads 1`, a process built as serial, the
  serial phase of a cycle at more threads) hands its messages to this stream as it makes them,
  in the order they would have had otherwise, so one that reports for a long time without a
  `wait` shows its output while it runs and keeps it when killed (#483); processes evaluated
  in parallel hand theirs over when they suspend. The bytes written are the same at any
  `--threads` value and with `--shuffle`.
- A write error on standard output (the reader closed the pipe, a full disk) stops the run
  between cycles (a run still writing a second later exits at once): a line on stderr says so,
  the waveform and coverage files are finished as after Ctrl-C, and the exit status is 141 (128
  plus SIGPIPE, as a shell shows a process killed by it) whatever the run would have returned
  (#484). `--color=always` forces ANSI
  colour on a pipe (the test runner passes it when it wants colour); without it yavs
  follows the TTY/`NO_COLOR` rules of plan `yavs-diag`.
- Runtime errors (bound check, overflow, null access, delta limit, internal) are diagnostics
  on stderr like every other error: code `E3001`-`E3010` per kernel `ErrorKind` (`E9999`
  internal), the failing statement, one `called from here` label per calling subprogram,
  and a note `at TIME, delta N, in process PATH`. A subprogram the optimiser inlined into its
  caller still has its label (and its element in a `get_call_path`).
- `--message-format=json` (section 0) is accepted by `run` and `elab` too, and
  covers runtime errors with their VHDL stacktrace; the test runner does not pass it today.
- `--cov[=FILE]` (`run` only) writes the run's coverage database to `FILE` (default
  `yavs.ycov`, relative to the working directory) after the simulation, whatever its exit
  status; an elaboration failure writes none. The file is overwritten, never appended to:
  one database per run, merged afterwards with `yavs cov merge A B ... -o OUT`. `yavs cov report
  DB ... [--cobertura FILE.xml] [--html DIR [--root DIR]] [--exclude FILE] [--uncovered] [-W...]` prints a text summary on
  stdout and writes Cobertura XML. `--html DIR` writes a static HTML report (`index.html`,
  `src-N.html` per source file in name order; inline CSS and one inline script, no external
  resources, no timestamp, so equal databases give equal bytes; source paths shown relative to
  the current directory or to `--root DIR`); sources are read from the paths in the
  database at report time and an unreadable one is `E0113` with nothing written. `--uncovered` prints instead one exclusion row per item that
  never ran (`statement|branch INSTANCE ID FILE:LINE   # text`, `condition|expression INSTANCE HEX12.OCC ROW FILE:LINE   # input, value, result, condition`, `toggle PATH SCALAR rise|fall
  # never 0->1|1->0 (FILE:LINE)`, or `toggle PATH SCALAR   # never toggled` when both
  directions never happened); `--uncovered=yaml` prints the same items as a YAML exclusion file
  (`exclude:` list of `kind`/`instance`/`hash`/`line` or `signal`/`direction` maps); `--exclude FILE` takes the
  items such a file lists out of the summary and the Cobertura report and lists them under
  "excluded" (format and warnings `W0110`, `W0111`, `W0114`: `docs/coverage.md` "Excluding code");
  the file is read as YAML when its name ends in `.yaml` or `.yml` (any case), else as text rows,
  with the same meaning (`E0112` names the line of a bad entry).
  Exclusions apply when reporting; databases keep raw counts. `--cov-exclude-translate-off`
  (`run`) also leaves `translate_off`/`translate_on` regions out; unbalanced `-- coverage`
  pragmas are `W0112`/`W0113`. `--cov-kind=statement,branch,toggle,condition,expression` (`run`, default `statement,branch,toggle`)
  chooses what is counted (`condition` and `expression` are focused expression coverage, #688:
  one row per input and value of a boolean condition or logic expression with 2 or more inputs;
  `docs/decisions.md` "Condition and expression coverage"); `--cov-toggle-max=N` (default 65536) leaves signals with more
  `bit`/`std_ulogic` scalars out of toggle coverage with `W0115`. `--cov-test=NAME` (`run`)
  names the run's test in the database; the default is the `enabled_test_cases` field of a
  `-grunner_cfg=...` generic (`,,` is a comma), else `TOP` or `TOP(g1=v1,g2=v2)` with the `-g`
  generics in command-line order. `cov merge` keeps per item the tests that hit it (matched by
  name); `cov report --tests` prints instead one exclusion row per item that ran, with
  `# tests: A, B`; `yavs cov rank DB... [--exclude FILE] [-W...]` prints a greedy minimal set
  of tests reaching the same coverage (`  NAME  +NEW  SO_FAR/HIT` per pick, then `not needed:
  ...`) and per test its items and the items no other test hit. `yavs cov` exits 0, or 1 with `E0111` (a database cannot be
  read), `E0112` (an exclusion file cannot be read or has a malformed row) or `E0110` (a file
  cannot be written), or when `-Werror` makes a warning an error; `run` exits at least 1 when the database cannot be written (`E0110`). The
  database format (a `yavs-coverage 5` line, a tests section of `test`, id, name lines with
  ids numbering the names in sorted order, then one tab-separated item per line: kind `s` or
  `bN`, path, statement text hash, occurrence, count, tests, line, file, statement text for
  display; conditions and expressions: kind `cN` or `eN` (N = 2 * input + value), the same fields with the
  input's text before the display text (the result the input decides alone, a space, the whole
  condition); toggles: kind `t1` (0->1) or `t0` (1->0), path, scalar name, count, tests, line,
  file; tests is the comma-separated ids of the tests that hit the item, `-` for none) is
  yavs's own and may change with its version line; versions 2 to 4 are still read (2 and 3 without
  tests, 2 to 4 without conditions). Cobertura output has no toggles.

## 6. GUI and waves

The test runner's `-g/--gui` option (this describes the viewer path; `yavs-gui` exists
now and the last paragraph of this section is the plan for using it): the test runner runs `yavs run ...
--wave=<output>/yavs/<entity>.fst`, then opens the file in the viewer:
`--viewer` argument, then `yavs.viewer.gui` sim option, then the default FST
viewers the runner knows, found on `PATH`. `yavs.viewer_script.gui` is passed to the viewer as
`--script FILE`. Only FST is written; the test runner warns and ignores `--viewer-fmt`
values other than `fst`.

`yavs run --gui <arguments>` opens the run in the graphical debugger: it starts `yavs-gui
--yavs <this yavs> -- <arguments>` (the run arguments without `--gui`), which runs `yavs run
<arguments> --dap=stdio` and drives it. `yavs-gui` is looked for next to the yavs executable,
then on PATH; neither is error `E0122` (exit 2). The exit status is the GUI's. `--gui` and
`--dap` together are a usage error (exit 2).

Planned, test runner side (`yavs-gui` exists now; nothing here says the runner uses it yet): `-g` will run
`yavs run --dap=0 ...` and start `yavs-gui` attached to it. `--dap=0` makes yavs pick a free port, and its first
line on stderr is then exactly `yavs: DAP listening on 127.0.0.1:<port>` (implemented, section
5); the test runner reads that line to connect the GUI. `--dap=PORT` with a fixed port
prints the same line.

The runner package `vunit-yavs` (`python/vunit-yavs`) today: `-g` runs `yavs run --gui @FILE`
(`--gui` outside the response file, so the GUI gets the plain run's arguments).
`--yavs-debug[=PORT]` runs `yavs run --dap=PORT @FILE` (default 4711, the editor extension's
attach port; `0` a free one), reads the `DAP listening` line and prints the address on the
terminal, since the runner shows a test's output only when it fails. It runs one simulation:
`--yavs-debug` with `--gui`, `--elaborate` or `-p` above 1 is refused at the start, and a second
simulation of the run is refused before yavs starts. `elab` never gets `--gui` or `--dap`.

## 7. Environment

The test runner sets no yavs environment variables (only `MAKEFLAGS` under `--yavs-jobserver`, section 5). `YAVS_STDLIB` overrides the std/ieee
tree (section 3); `YAVS_STD_CACHE` names the directory of the shared std/ieee units, or `off`; `--std-cache` overrides it (section 3). Simulator hooks from the test runner packages
can add `elab_flags`, `run_flags` and run environment; the test runner applies
`elab_flags` to both `run` and `elab`, and `run_flags` only to `run`. `YAVS_HUGE_HEAP=0` stops yavs
from asking for transparent huge pages on its heap at start (Linux with glibc and the system's
transparent huge pages in `madvise` mode only; on any other system it does nothing). `--wave` ends a waveform block by size only, so the
file's bytes depend on the recorded changes alone; `YAVS_WAVE_FLUSH=5s` (or `500ms`) also ends a
block after that much run time, so a killed run leaves more of the file (the bytes then depend on
the run's speed); `never` is the default; any other value is error E0801.

## 8. Not covered yet

Coverage through the test runner: the runner still reports `supports_coverage() == False`;
users pass `--cov=FILE` in `yavs.sim_flags` and merge the databases themselves (section 5).
