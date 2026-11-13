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
  - A relative FILE is relative to the current directory. A missing,
    unreadable or not UTF-8 FILE is error E0101 and a usage error (exit 2); the note gives the offset of the first bad byte.
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
  built in: `yavs 0.9.0 (compiled tier: cranelift)` (a build with the `clift` feature) or `yavs 0.9.0
  (interpreter only, no compiled tier)` (a build without it fails `--tier=auto`, `clift` and `clift-auto` with E0120).
  Builds before the LLVM tier was removed (2026-11-08) could also say `llvm`. The release archives are built with `clift` only, `compiled tier: cranelift` (#780): one per platform, and for x86_64 a second `-compat` one.
  The x86_64 release archives without `-compat` are built for x86-64-v3: on a CPU without it every command exits with status 1
  and a plain message on stderr (`error: this build of yavs needs a CPU with x86-64-v3 ...`, naming the missing
  features, the `-compat` archive and building from source), before the command line is read. It has no code and no `--explain`
  text: it runs before anything else can (`docs/decisions.md` "x86-64-v3 baseline"). A test runner that checks
  `yavs --version` refuses the executable, and the message reaches the terminal.
  A second line says which CPU level the binary was compiled for, chosen from the compile-time
  target features (so it is true of the binary, not of its archive name): `Build: x86-64-v3, AVX2 support`
  when AVX2 is enabled, `Build: x86-64 baseline` for x86-64 without it (the `-compat` archives and builds
  from source), `Build: aarch64` on AArch64. A third line carries the copyright notice,
  `Copyright (c) 2026 Sebastian Abrahamsson`; a consumer that embeds the version takes the first line only. The test runner takes the first
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
| `supports_coverage` | True | `enable_coverage` adds `--cov-file` with the kinds of `yavs.cov_kind`, `--yavs-cov-kind` or `--yavs-cov=LETTERS`, `merge_coverage` runs `cov merge` and `cov report` (section 5) |
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
  has a `format` number in the index (4). A library written by a yavs with another number is refused with
  `L0004` naming the directory; delete it and analyze its files again (libraries are not converted). The
  index holds the absolute form of the name a file was last analyzed by (symlinks and `..` as written), and
  one entry per file: analyzing it by another name replaces the entry. A symlink of another name to a file
  that was edited since is not recognized and stays a second entry.
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
  for writing. The only things they write into a library directory are the compilation cache
  entries of the work library (below, in `<work dir>/yavs-cache/`) and the analysed-unit entry
  (`yavs-cache/units/`) of a unit they had to analyse from source, into that unit's library,
  best effort and by atomic rename (or by creating the file whole), so any number of them may
  run at once against the same libraries; `analyze` writes unit entries the same way.
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

- Units analyzed again: the library holds the last analyzed unit of each name, and any primary unit replaces another of its name. `run`/`elab` use those units: a unit analyzed before something it depends on was replaced is analyzed again against the new one (LRM 13.5, no diagnostic), a unit that no longer fits fails the run with that analysis's error, and the old copy in a file that lost a unit is not analyzed (`docs/decisions.md` "A run uses the units the library holds").

### Compilation cache

`yavs elab` and `yavs run` keep what analysis of the design's closure produced in the work
library's directory, `<dir>/yavs-cache/sem-<key>.img` (user decision 2026-10-19, `docs/decisions.md`
"Compilation cache"). The next `elab`/`run` of the same closure reads it instead of parsing and
analyzing the sources again; the output, the diagnostics (warnings included) and the exit status are
identical to a run without it, at any `--threads`.

- The key is a hash of the yavs build, `--std`, `--relaxed`, `--psl`, `--encoding`, `-D` and every file of the
  closure (library, path, recorded content hash, standard, and the units it still holds if a later file took some). Editing a file (or any file it depends
  on) is found by the content hash check of the closure (`L0034`, re-analyze it); an entry also records
  the bytes of the built-in `std` and `ieee` files it was made from and is used only if they are
  unchanged. Entries of another
  yavs build, another format version or a damaged file are ignored, never an error. An error-free
  analysis is stored; a design with errors never is.
- Entries are written to a temporary file and renamed into place: concurrent runs need no lock and
  never see a partial entry. A library directory keeps at most 4 GiB of entries (and 65536 files); the
  limit is soft by what parallel runs write in the 5 s between two looks at the directory. At the limit,
  entries not used for an hour go first, oldest first; then, most recently used first, entries not used
  for 10 s. A suite larger than the limit runs through its designs in order, and this keeps the
  designs that fit warm, where removing the least recently used would remove each entry just before
  its next use (#1027). A suite far over the limit is not cured by this: measured with the 4 GiB limit, later runs cost 3 percent fewer instructions than the first at about 2.4 times the limit, 1 percent more at about 3.6 times and 7 to 8 percent more at about 6 times (#1034). With nothing older than 10 s, or for an entry bigger than the limit, the new
  entry is not stored; a new version of an entry that is there always goes in. A library
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
  writes, `--no-cache`, `yavs cache clean`). It counts toward the limit of the directory.
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
  messages and waveforms are identical with and without it. It counts toward the limit of the directory.
- A closure's first run also uses what the files it starts with give once for all closures of that
  prefix, kept in the cache of the library of the prefix's last file: the lowered library
  (`lib-<key>.img`: the packages and every subprogram they declare that does not depend on the design),
  the state of its first packages after initialisation (`libinit-<key>.img`) and the optimised copies
  of its functions (`libopt-<key>.img`). The library is made when four different closures have asked for the prefix, by
  one run at a time (`<kind>-<key>.making` while it does), and only when no package body follows the
  prefix, nothing was instantiated at elaboration time and the run has no coverage counters and is not
  `--tier=clift`; otherwise the run lowers as before. Results, messages and waveforms are identical with
  and without it. These entries count toward the limit of the directory.
- With `--tier=clift` or `--tier=auto` the machine code of every function Cranelift translates is kept in
  `<dir>/yavs-cache/obj/clif-<key>.img` of the work library (the function's translated body, the target and the
  yavs build; #481, #505); a later run that makes a function with the same key links it instead of compiling it.
  Same rules: identical results, damaged or foreign entries ignored and compiled again, atomic writes, nothing
  stored into a directory that cannot be written, `--no-cache`, `yavs cache clean`. The directory keeps at most
  16384 objects and 256 MiB, apart from the 4 GiB of entries, under the same rule at the limit. (The removed
  LLVM tier kept its modules there as `obj-<key>.img`.) `route-<key>.img`, one per design, holds how each compiled function
  first called the others, so that an unchanged design makes the same bodies in every run (#1035).
  `clifpre-<key>.img`, one per function, names the `clif-<key>.img` of its code and the symbols it
  refers to, under a key of everything the code depends on, so a warm run links without translating.
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
  `analyze` writes the entry of every unit it analyses from source (under its library session; the units
  of other libraries and of `std`/`ieee` by atomic rename, no lock). `elab` and `run` write the entry of a
  unit they had to analyse from source (its entry was missing, stale, or made under the other `--relaxed`
  where the unit asked for it) the same way, without a lock, into the unit's library or the shared store,
  so the next closure that needs it replays it (user ruling 2026-11-07, `docs/decisions.md` "Analysed-unit
  cache"). Two writers of one entry write the same unit; a reader checks every entry it uses, so one that
  lost a race with an edit is a miss. A library directory that cannot be written is read and never fails. `analyze --no-cache` neither reads nor writes them; `yavs
  cache clean` removes them. The directory keeps at most 16384 entries and 512 MiB, apart from the 4 GiB
  of entries, under the same rule at the limit (a read does not refresh a unit's time, so its age is the
  age of the entry). A library directory therefore holds at most about 4.75 GiB of cache.
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
          [--cov=LETTERS] [--cov-file=FILE] TOP [-gNAME=VALUE ...] [+NAME[=VALUE] ...]
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
- `--stop-time=TIME` (e.g. `10ms` or `10 ns`; a bare number is `E0105`), `--threads=N` (at least 1; 0 is a usage error, exit 2, #839) or `--threads=auto` (below), `--shuffle`,
  `--vhpi=LIB[:ENTRY]`, `--dap=PORT`: not passed by the test runner, used through
  `yavs.sim_flags` or simulator hooks. `--load=LIB` is taken by `yavs elab` as well as `yavs run`
  (#943); `native_library_hooks` passes it to both through `elab_flags`.
- Stuck and hung simulations (#582), `run` only, all as diagnostics (code, labels, notes, help; JSON
  with `--message-format=json`; `yavs --explain CODE`):
  - `E3002` (delta cycle limit) names the processes and signals of the loop found from the drivers
    and waits at the time of the error (two processes waking each other, a process waiting only on a
    signal it assigns, a combinational loop through concurrent statements, `wait for 0 ns;` in a
    loop), labels their places and says how to raise the limit. It also labels the other processes
    of the last delta and the declaration of every signal with an event in the last 16 deltas, with
    its value for a short logic signal (at most 8 such labels, then a count). `--max-deltas=N`
    (default 10000, at least 1) sets the limit. Nothing is recorded during the run.
  - `W3101` is the hang report: time and delta, how many processes wait for a timeout, for signals
    that can still change, or can never run again; for each of those processes (at most MAX per
    part) what it waits for, labelled at the statement it stands at when it runs interpreted (else
    where the process starts), and for each signal that nothing can change its value, its last event
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
  - A session (`--dap`, `--interactive`, `--do`) takes `--hang-report`, `--assert-summary` and
    `--activity[=FILE]` too, printed as console output at the end of the run, in text.
  - The console commands are in the user guide, generated from the table of `yavs-dap`
    (`docs/console-commands.inc`). `wave save` and the end of a session write
    `<work library>/yavs-wave-layout/<top in lower case>.txt`, one dotted name per line, `#` comments.
  - `--assert-summary` adds to stderr, after the reports and whatever the outcome: a table of the
    reports and failed assertions per severity and source line with the first message, or with
    `--message-format=json` one object `{"assert_summary": {"totals": {SEVERITY: N}, "places":
    [{"severity", "count", "file", "line", "message"}]}}`. Reports disabled with
    `SetVhdlAssertEnable` are not counted; `ClearVhdlAssert` does not clear it. Reports and the exit
    status do not change. `--activity[=FILE]` adds to stderr the top 20 processes by runs and signals
    by cycles with an event, with FILE also JSON `{"processes": [{"name", "runs", "place"}],
    "signals": [{"name", "events", "place"}]}` of all of them. Neither costs anything unless given.
  - `W3102`: a process whose body has no wait statement, no sensitivity list and no call of a
    procedure that waits, printed before time 0. It never gives control back, so time cannot
    advance (only `stop` or `finish` ends the run).
  - `--shuffle[=SEED]` (seed 1 without a value) prints ``note: process order is shuffled with seed N;
    replay it with `--shuffle=N` `` on stderr at the start, and the runtime diagnostics of a shuffled
    run repeat the seed. That is the only run-time randomness; there is no time-based seed.
- `--no-collapse-ports` (`run` and `elab`): keep every port its own signal. By default the
  formal of a port whose actual is a signal name (a whole signal, record elements, an element or a
  slice), with no conversion and no force or release of the port, is an alias of its actual: no
  separate signal and no copy per delta. That holds for `in` ports, for `out` and `buffer` ports
  that are the only source of their actual, and for `inout` ports (their drivers drive the actual;
  an element without a driver keeps the port's default as a source); a port with a user resolution
  function or on a guarded, forced or whole-resolved actual stays apart. Output, waves and
  `'path_name` are the same either way; a forced port, and every port under `--dap` or `--vhpi` (a
  debugger or plugin may force or deposit any port), stay apart. For comparing against the
  uncollapsed design.
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
  calls (the subprogram exactly at every call with `--tier=auto`), so short
  activations may show none, a statement is where the interpreter last looked, and compiled
  code has no `lines` rows. A small subprogram the optimiser inlined counts as its caller in
  `subprograms`, but its statements are `lines` of their own function. The sampler is a thread
  that sleeps between samples: no signal or timer is used. Time in plugin callbacks (`--vhpi`) is
  in the kernel share. With `--dap` it is `E0125` (exit 2). Off by default and
  then free: no process is wrapped and no thread exists.
- `--load=LIB[:args=LAYOUT]` (repeatable; `LAYOUT` is `length`, the default, or `fat-pointer`, see the VHPI guide; another name is `E0136`, exit 2; libraries load before elaboration, in `yavs elab` too, since a signal's initial value may call them): load a shared library for VHPIDIRECT foreign
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
- `--exit-on-stdin-close` (`run`): a thread reads standard input and, when it is closed or ends, writes
  `yavs: standard input closed (the parent ended): stopping the run` to stderr and ends the process with status 130, also
  while the run waits for a debugger, analyses or elaborates. For a parent that holds a pipe to the child and may die
  without a word (SIGKILL, TerminateProcess, a crash): the kernel closes the pipe on Linux and on Windows alike, so no
  signal, process group or polling is involved. Waveform and coverage files are not finished (a Ctrl-C does that). Bytes that
  arrive on the way are dropped. Standard input is taken for the watcher alone: file descriptor 0 and the Win32 standard input
  handle point at the null device for the design (`textio` `input` is at its end) and for every library, plugin or embedded
  interpreter, because on Windows a pending read of a pipe in one thread makes `GetFileType` on the same handle in
  another block, which hung the start-up of an embedded Python. Not with `--interactive`
  (clap usage error, exit 2). With `--dap=stdio` it does nothing: the protocol stream is standard input, and its end
  already ends the session. Not covered: a child that was started with an inherited terminal or `/dev/null` as standard
  input (nothing closes), and processes yavs starts itself (`--surfer`'s viewer). #940.
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
  `E0199` not implemented (exit 2). A port that cannot be opened is `E0701` (exit 2). Custom requests, served while stopped: `yavs/runFor` and
  `yavs/runUntil` (argument `time`, a TIME literal such as `"10 ns"` or `"10ns"`, relative to the
  current time or absolute), `yavs/stepDelta` and `yavs/stepTimestep` (no arguments); each
  answers, sends `continued`, and the next `stopped` event's `description` gives the time and
  delta. Signal breakpoints are DAP data breakpoints (`supportsDataBreakpoints`; `dataId` is
  the signal's lower-case path, stored as `:top:u1:s`) and the custom request `yavs/setSignalBreakpoints`
  (`breakpoints: [{signal, value?}]`, `frameId` for names seen from a process; it replaces its
  own set): a stop after the cycle in which the signal changed (to `value`), reason `data
  breakpoint`, `text` `PATH = VALUE` (a dotted path, as in thread and frame names and the answers below; `dataId` keeps the stored `:top:u1:s` form, which a client treats as opaque). `yavs/force` (`signal`, `value`, `mode` `in` or `out`,
  default `in`), `yavs/deposit` (`signal`, `value`) and `yavs/release` (`signal`, `mode`; no mode ends a force of either) act on
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
  watched signal's region) or by path, with predefined operators, numeric comparisons of `unsigned`
  and `signed` and `to_integer`; one that does not parse, uses something unsupported or compares a
  signal with a literal that is no value of its type, names nothing visible, mixes operand types or is
  not boolean is `verified: false` with a `message` (the check runs when the breakpoint is set; a
  variable of a process is known by name only, so a verified breakpoint names such variables in its
  `message`), an error while evaluating at a stop (a value-dependent one) a stop with reason
  `exception`. Physical values, `time` among them, are shown in the largest unit that holds them
  whole (`10 ns`). `hitCondition` counts the hits where the condition
  held. `evaluate` takes a local or signal, a part of one (field, element, slice, `.all`) or any such
  expression. `completions` (`supportsCompletionsRequest`, at a stop) answers targets with `label` and
  `length`, the characters before the cursor it replaces. Access values and protected objects expand
  through `variablesReference`; what is behind them is not written by `setVariable`. Variables carry
  `evaluateName` (a signal's dotted path, a local's name, a part's). `initialize` answers `yavsViewer: true`
  in a session with `--surfer`. `--dap`, `--interactive` and `--do` with a wave file imply `--wave-variables`.
  A failure before a session starts under `--dap=stdio`, and over TCP once a client connected: `initialize`
  succeeds, `launch` or `attach` get an `output` event of category `stderr` and a failed response with
  the rendered diagnostic as `message`, other requests fail with it; the process exits with its status at
  `disconnect`, at the end of input, or after 30 s. The diagnostic also goes to standard error. Standard
  input being a terminal skips all of this. Before `exited`, a session that is no console sends an `output`
  event giving the exit status.
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
- `--tier=interp|auto|clift|clift-auto` (`run`; default `auto` in a build with the `clift` feature unless `--dap` is
  given, else `interp`, quietly: no error, no warning; user, 2026-10-22, #510): `clift` compiles every function of the
  design with Cranelift before time 0 and runs that code; `auto` starts in the interpreter
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
  slots. `--tier-max-size=N` (default 50000): with `clift` or `auto`, a function of more
  than N IR instructions stays interpreted (user, 2026-10-19, #126). `--tier-compile=wait|background`
  (default `wait`; user, 2026-11-07, #1123): with `auto` (and `clift-auto`), where hot functions
  compile. `wait`: the tier-up thread compiles only while the simulation waits for it at a pause point (the
  start of a delta cycle, the interpreter's periodic looks inside a long process run, a step that compiled
  code hands the interpreter), so no compile runs alongside the simulation threads. The tier-up thread
  still prepares functions (optimising ahead of the compile, analyses) alongside the run, so a run can use
  somewhat more than `--threads` cores for short periods (measured, 2026-11: on average about a tenth of a
  process's CPU time, up to 30 percent in a short test), and it has further helper threads (sampler, signal
  waiter); `--tier=interp` starts no tier-up thread. Compiled code in a loop that
  never leaves it reaches no pause point, and a compile waits until it does. The compiles are paced: their
  compile time, each priced at what compiling it was measured to cost, stays within a quarter of the
  simulation CPU time the looks covered (the whole of it once the looks hold a second of simulation work), and a function whose whole compile does not fit waits; of those the
  count rule asked for, links from the cache come first, then the most seen per compile time. In the first run of a program (no profile of an earlier run), until the looks hold a second of
  simulation work, a function the count rule asks for and the cache does not hold compiles only once it was seen
  interpreted for its modelled compile time. In the runs after it, such a function compiles only when its share of
  the looks, over the rest of the run that the last run's length predicts, times what compiled code saves and the
  few runs the cache serves, pays for its compile; what the earlier runs saw of it counts too, and a function
  held back in the run before (the program's profile keeps them) compiles anyway. `background`: the tier-up thread compiles alongside
  the run at the lowest OS priority (Linux nice 19, Windows `THREAD_PRIORITY_LOWEST`; nothing else
  elsewhere), one busy thread more than `--threads` while it compiles. What is hot is judged the same way
  in both (the policy's clocks count the simulation threads only), but `wait` paces its compiles (above), so
  a short run, or one whose count asks for many functions, compiles fewer. The output is the same. The other tiers compile nothing during the run and ignore the option, as they do
  `--tier-heat`. `--tier-heat=BASE,PER_INSTR,PERIOD`
  (hidden, for tests and tuning): with `auto`, a function is hot once a sample every PERIOD
  microseconds has seen the interpreter in it for BASE microseconds plus PER_INSTR nanoseconds
  per IR instruction of it (PERIOD `0`, as in `1,0,0`: from its first call or jump). `auto`, `clift`
  and `clift-auto` need a yavs built with the `clift` cargo feature: otherwise asking for them explicitly is error
  `E0120` (exit 2). `llvm` and `auto3` were removed with the LLVM tier (user, 2026-11-08): asking for them is `E0120`
  in every build, before any analysis, with a help line naming the tiers to use. `--help` for `--tier` and `--explain E0120` name the tiers the executable
  has (#845, #850); a
  design the tier cannot compile (or a compile thread that cannot start) is `E0121` (exit 3, a
  bug in yavs; `E0127`, the LLVM library that could not be loaded, is retired). `clift-auto` is another name for
  `auto`. `--verbose` prints, on stderr, a note per function left to the
  interpreter and how many instructions were compiled (`clift`), or a note per hot function as
  the compile thread takes it (`auto`, so their order and number depend on timing). The test
  runner passes neither; use `yavs.sim_flags`. It passes `--tier-compile` from the sim option
  `yavs.tier_compile` (before `yavs.sim_flags`, which win).
- Threads: yavs defaults to `--threads=1`. The test runner already runs `-p N`
  simulations in parallel and passes a thread count only when the user sets one: the sim
  option `yavs.max_threads` (or the runner argument `--yavs-max-threads N` for every test; both
  take a number or `auto`) becomes `--threads=N` before the raw flags, so `--threads` in
  `yavs.sim_flags` still wins; none under `--dap` or for `elab`. With the runner's
  `--yavs-jobserver` and `-p N` above 1 (`-p 0` is one job per CPU), the runner starts a token server and every `yavs run`
  gets `--threads=auto` (or `yavs.max_threads` when set, before the raw flags) and its own
  `YAVS_TOKENS`: the runner lends the CPU slots no test uses as tokens, at most
  `min(N, 8) - 1` per test with `auto` and `yavs.max_threads - 1` with a number, breadth first,
  newest test first, and never takes back a token in use (python/vunit-yavs/README.md "Token
  server"). Before 2026-11-07 the default under the server was `--threads=min(N, 4)`.
- `--threads=auto` (#884): at most 8 threads, and no more than the CPUs the process may run on
  (affinity and quota, asked when a batch first wants the threads; one where the system does not
  tell). A design with fewer than 8 processes that may run in parallel (not serial, not
  postponed) runs on one thread with no trial. Otherwise the first trial starts 10 ms after the
  build (a shorter run pays nothing for it), and the kernel judges the threads in windows of
  about 25 ms of wall time while they are on (the clock is read about once a millisecond, as
  often as the delta cycles allow, so these times are lower bounds, coarser for a design whose
  delta cycles each take longer than that): a window
  wins when the time its pooled batches saved (their estimate inline, corrected by timed probe
  batches, less their measured time) is at least 4 percent of it; one window in which no batch
  wanted the threads (such a window ends after 5 ms), two losing windows in a row, or four once a
  window of this or the last run won, turn them off (the code paths of `--threads=1`, tokens given back), and a trial comes
  again after 1 s, doubling up to 16 s (0.1 s once a window won). The worker threads start when
  the first batch goes to them. What the last run of the same command line found (gained or not)
  is kept in the work library's cache, kind `thr` (not with `--no-cache` or without a work
  directory). The command line is the arguments with response files read, where a `runner_cfg`
  generic counts without its `seed` and `output path` fields (the `--tier=auto` profile is
  keyed alike). After the first window that wins, and then after every 16 that win, one window
  runs on the other thread count (all of them, or half) and the count whose pooled batches ran
  faster against the inline time of that window's timed probe batches is kept, all of them only
  when 5 percent faster: hardware threads of one core or a shared cache make some designs faster
  on fewer threads. When the threads start (and at each trial) the first two batches of 8 or more
  processes run on one thread with every process timed, so the estimates are known from the
  start. A run after one that did not gain starts on one thread with a first trial after
  4 s, one after a run that gained starts with the threads on and retries after 0.1 s. `--verbose` prints a note
  `note: --threads=auto: ...` on stderr at the end. With a token server it connects as for
  `--threads=N` above 1. Under `--dap`, `--interactive` and `--do` it is `W0109` as any value
  but 1. Output and exit status do not depend on any decision. `YAVS_COUNTERS=1` prints
  `threads_windows` and `threads_gained`.
- Token server (#801): with `--threads=N` above 1 (and not `--dap`), yavs reads
  `YAVS_TOKENS=<port>:<secret>` at the start of `run`. The simulation owns one implicit token and
  runs at most N - 1 more threads, only on tokens the server lends. Protocol, lines of ASCII
  ending in `\n` over TCP to `127.0.0.1:<port>`:
  1. yavs connects and sends `<secret>`; the server answers `0 0`, or closes the connection
     when it does not know the secret (a reset counts as a close: error `E0128`).
  2. yavs sends `<want>` (a decimal count, at most N - 1) when the number of tokens it could use
     changes: N - 1 when a group of processes is worth the threads, `0` after 64 groups in a row
     on one thread. Its k-th such line is answered at once with `<k> <lent>` and again whenever
     the server lends it more (`<k> <lent>`, `lent` at most `want`). yavs uses the `lent` of the
     answers to its latest line only, reads them without blocking only when a group of
     processes is worth the threads, and runs a group on one thread while none is lent.
  3. Closing the connection (any exit of yavs, also a crash or a kill) gives every token back.
     A lower `<want>` gives back the tokens above it. The server never takes back a token.
  A value that is not `<port>:<secret>` and a secret the server does not accept (a closed
  connection or another answer) are error `E0128` at the start of `run`, before analysis (exit 2);
  the message never contains the secret. A port where nothing listens (connect timeout 10 s), a
  server that does not answer within 10 s and a server that goes away during the run are warning
  `W0118` (one warning; the run is on one thread from then on, tokens only speed it up). With
  `--threads=auto` it connects too, with N = 8; on a host where the run may use fewer CPUs it
  asks for no more tokens than CPUs less one, and none on one CPU. Without `YAVS_TOKENS`, or with `--threads=1`, nothing changes. Output and exit status do not depend
  on the tokens.
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
- `--cov=LETTERS` (`run` only) turns coverage on and selects the kinds, in any order: `s` statement,
  `b` branch, `c` condition, `e` expression, `t` toggle, `f` fsm, `a` assert, `m` combination
  (`--cov=sbt`); a repeated letter is harmless. No kind is on by default (user, 2026-11-01):
  bare `--cov`, `--cov=` or a letter that is none is `E0140` before analysis (exit 2), listing every
  letter and a command to copy; a value that looks like a file name also says that the database goes
  in `--cov-file`. `--cov-kind=LIST` is the long form with words, also turns coverage on, and is not
  given with `--cov=` (`E0140`). `--cov-file=FILE` (needs coverage on, else `E0140`) names the database
  (default `yavs.ycov`, relative to the working directory), which is written after the simulation, whatever its exit
  status; an elaboration failure writes none. The file is overwritten, never appended to:
  one database per run, merged afterwards with `yavs cov merge A B ... -o OUT`. `yavs cov report
  DB ... [--cobertura FILE.xml] [--json FILE] [--lcov FILE] [--sonarqube FILE] [--html DIR] [--root DIR] [--kind LIST] [--total] [--fail-under PCT|KIND:PCT,...] [--below PCT] [--exclude FILE]... [--config FILE] [--uncovered] [-W...]` prints a text summary on
  stdout and writes Cobertura XML. The summary is a table `Kind  Items  Hit  Missed  Coverage` of the kinds the database holds with a `total` row (`no coverage items` when it holds none) and a table with a line per instance or package;
  then the number of items not covered and, with exclusions, the number excluded. `--detailed` (conflicts with `--total`, `--uncovered`, `--tests`, `--exclusions`) replaces the counts with a section per path, `=== PATH  (FILES) ===`: a table per kind with every item (`Line  Item  Hits  Status`, status `covered`, `MISSED` or `excluded`, a branch named by its arm in the source:
  `if C`, `elsif C`, `else`, `when CHOICES`, `(no arm taken)`, `loop body`; a toggle is `SIGNAL rise (0->1)` or `fall (1->0)`, bits of one direction and count on one row as `s(0..7)`), per condition or expression a line when fully covered, else its input and row tables (`Row  Hits  Target  Other inputs must be`), per decision a table of its combinations, the text that excludes each missed item, then per finding a note in the form of the diagnostics, worst first, and a closing line. `--below=PCT` limits the table and the sections to paths under PCT, worst first, with a line counting the rest; totals and gates are unchanged. This text is for reading and may change: `--uncovered` and `--tests` stay one row per item. Source paths in the text reports, `--uncovered`, `--tests`, the HTML report
  and the exports are relative to the current directory or to `--root DIR` (a file outside it
  keeps its path), and so are the paths in every export, Cobertura included. `--kind=LIST` (`statement`, `branch`,
  `toggle`, `condition`, `expression`) keeps only those items in every report. `--total` prints
  only the percentage of items that ran over all items, after exclusions and `--kind`, rounded
  down to one decimal (`74.3%`); it conflicts with `--uncovered` and `--tests`. `--fail-under
  PCT` (0 to 100, decimals allowed) exits 1 with `E0114` (`coverage 70.0% is below the
  required 80.0%`, after the normal output) when that total is lower; `--fail-under=KIND:PCT,...`
  (beside or instead of the plain number) does the same per kind, counted as `--kind=KIND --total` counts, and the message names every
  requirement that failed (`branch coverage 50.0% is below the required 80.0%`); a PCT that is not a number
  in range, an unknown kind or a repeat is a clap usage error with exit status 2. A kind `--fail-under` names with no items is `E0115`. A total with no item to count (`--total`
  or `--fail-under` with a `--kind` the database lacks) is `E0115`, exit 1. `--cov-embed-sources` (`run`, needs coverage on, else `E0140`) stores the sources in the database. `--html DIR` writes a static HTML report (`index.html`,
  `src-N.html` per source file in name order; inline CSS and one inline script, no external
  resources, no timestamp, so equal databases give equal bytes; source paths shown relative to
  the current directory or to `--root DIR`, as in the text reports); embedded sources are used, others are read from the paths in the
  database at report time and an unreadable one is `E0113` with nothing written; different embedded text for one path across merged databases is `W0121`, the last wins. `--uncovered` prints instead one exclusion row per item that
  never ran (`statement|branch INSTANCE ID FILE:LINE   # text`, `condition|expression INSTANCE HEX12.OCC ROW FILE:LINE   # input, value, result, condition`, `toggle PATH SCALAR rise|fall
  # never rise (0->1)|fall (1->0) (FILE:LINE)`, or `toggle PATH SCALAR   # never toggled` when both
  directions never happened); `--uncovered=yaml` prints the same items as a YAML exclusion file
  (`exclude:` list of `kind`/`instance`/`hash`/`line` or `signal`/`direction` maps); `--exclude FILE` (repeatable: the rules of all files add up, in the order given) takes the
  items such a file lists out of the summary and the Cobertura report and lists them under
  "excluded" (format and warnings `W0110`, `W0111`, `W0114`: `docs/coverage.md` "Excluding code");
  the file is read as YAML when its name ends in `.yaml` or `.yml` (any case), else as text rows,
  with the same meaning (`E0112` names the line of a bad entry). `file` rows and entries take a plain
  directory; `files` and `files_exclude` (YAML under `kind: file`, one word or a list; text rows
  `files P... [files_exclude P...]`, `files_exclude P...`) use the matching of lint waivers
  (`yavs_diag::files`), relative to the exclusion file, and a pattern that covers no source file of the
  database is `W0111`.
  `--config FILE` reads YAML with `exclude` (a word or list of exclusion files, added before those of
  `--exclude`), `root` (both relative to the file) and `kind`; `--root` and `--kind` win; no file is
  read without the option; a bad or unreadable file is `E0138`.
  In-source `-- yavs cov exclude state NAME` and `transition FROM -> TO` (`*` at either end), before the `case` on a state register or one of its `when` alternatives, waive that state machine item in every instance.
  In-source `-- yavs cov exclude row -0-` and `row INPUT = VALUE` (0, 1, false, true) waive one condition or expression row; a
  pattern without `row` always means combinations. Both forms are stored in the database, and both follow `E0137`.
  A pattern may be followed by its inputs, `01 (a, b)`, `row -0- (a, b, c)`: checked at analysis, a mismatch is `E2022`.
  Rows `library GLOB`, `entity GLOB` (`NAME` or `LIB.NAME`) and `process INSTANCE LABEL` (YAML: `kind` with the
  same keys, `instance` and `process` for the last) need the database's unit and process fields; a database
  without them is `E0112`. `--uncovered` adds `[process LABEL]` to the comment of an item in a labelled process.
  Exclusions apply when reporting; databases keep raw counts. `--cov-exclude-translate-off`
  (`run`) also leaves `translate_off`/`translate_on` regions out; unbalanced `-- coverage`
  pragmas are `W0112`/`W0113`. `--cov-kind=statement,branch,toggle,condition,assert,expression,combination,fsm` (`run`, no default)
  chooses what is counted (`condition` and `expression` are focused expression coverage, #688:
  one row per input and value of a boolean condition or logic expression with 2 or more inputs;
  `docs/decisions.md` "Condition and expression coverage"); `--cov-toggle-max=N` (default 65536) leaves signals with more
  `bit`/`std_ulogic` scalars out of toggle coverage with `W0115`. `--cov-test=NAME` (`run`)
  names the run's test in the database; the default is the `enabled_test_cases` field of a
  `-grunner_cfg=...` generic (`,,` is a comma), else `TOP` or `TOP(g1=v1,g2=v2)` with the `-g`
  generics in command-line order. `cov merge` keeps per item the tests that hit it (matched by
  name); `cov report --tests` prints instead one exclusion row per item that ran, with
  `# tests: A, B`; `yavs cov rank DB... [--exclude FILE]... [--config FILE] [-W...]` prints a greedy minimal set
  of tests reaching the same coverage (`  NAME  +NEW  SO_FAR/HIT` per pick, then `not needed:
  ...`) and per test its items and the items no other test hit. `yavs cov` exits 0, or 1 with `E0111` (a database cannot be
  read), `E0114`, `E0115`, `E0112` (an exclusion file cannot be read or has a malformed row), `E0138` (`--config`) or `E0110` (a file
  cannot be written), or when `-Werror` makes a warning an error; `run` exits at least 1 when the database cannot be written (`E0110`). The
  database format (a `yavs-coverage 13` line, a tests section of `test`, id, name lines with
  ids numbering the names in sorted order, with `--cov-embed-sources` a `source`, path, text line per source file (`\\`, newline, return and tab escaped), then per item an `at` line after it (line, column, byte start, byte end, hex hash of those bytes, the arm around it `HASH.OCC.ARM` or `-`, its statement `HASH.OCC` or `-`, what the other inputs must be, the toggle's last level, `d` when a directive can waive it) and one tab-separated item per line: kind `s` or
  `bN`, path, statement text hash, occurrence, count, tests, line, file, statement text for
  display; conditions and expressions: kind `cN` or `eN` (N = 2 * input + value), the same fields with the
  input's text before the display text (the result the input decides alone, a space, the whole
  condition); toggles: kind `t1` (0->1) or `t0` (1->0), path, scalar name, count, tests, line,
  file; combinations: kind `mS` (S = the state: two bits per input, 0 not evaluated, 1 false, 2 true),
  the condition fields, the input texts separated by U+001F before the display text, and last the
  in-source waivers (`-` for none; else rule, reason, file and line separated by U+001F, waivers
  by U+001E); tests is the comma-separated ids of the tests that hit the item, `-` for none) is
  yavs's own and may change with its version line; versions 2 to 11 are still read (2 to 11 without embedded sources and `at` lines, 2 and 3 without
  tests, 2 to 4 without conditions, 2 to 5 without combinations and in-source exclusions, so
  `cov report --exclusions` says it has no record); any other `yavs-coverage N` line is `E0111` with a note
  naming N, the versions this yavs reads, and to re-run or use the yavs that wrote it (`cov merge`, `report`, `rank` alike). Cobertura output has no toggles and no
  combinations.

  `--cov-kind=combination` (#883) counts, per condition or logic expression with two or more
  inputs, each combination of input values that can occur (an input a short-circuit operator
  skips is "not evaluated", distinct from false; at most 8 inputs and 512 combinations, else
  warning `W2023`). `-- yavs coverage exclude ...` comments (`-- yavs cov` is the same) waive
  combinations of the decision, `elsif` or assignment right after them; they are checked at analysis
  by `analyze`, `elab` and `run`, with or without coverage: `E2020` to `E2025` are errors,
  `W2020` to `W2023` warnings (`docs/coverage.md`). `--coverage-directives=apply|ignore`
  (`analyze`, `elab`, `run`) chooses whether they are read at all; with `ignore` they are neither
  checked nor applied and a note counts them. Waived combinations are out of the totals like other
  exclusions, their raw counts stay in the database, and `cov merge` of a database that waives
  one with a database that requires it is `E0137`. `cov report --exclusions[=text|json]` prints
  the exclusion report of in-source directives and `--exclude` rows together (JSON `"version": 1`),
  and `--html` adds `exclusions.html`; exclusion files take `combination INSTANCE HEX12.OCC
  PATTERN` rows.

### Paths in output

Every hierarchical path yavs writes itself is dotted, through one helper (`yavs_diag::dotted`): `tb.gen(1).u.p`, the index of a for-generate in parentheses as in `'path_name` (the label `gen(1)` is one level; `gen(':')`, an index of a character type, keeps its literal; an extended identifier keeps what it holds). That holds for report and assert lines (`    Process tb.gen(1).u.p`), `Function resolution of tb.s`, runtime errors and all diagnostics with their labels and notes, `--print-hierarchy` (a region in the instance form, `tb(a).gen(1).u@leaf(a)`; an object with its kind, `tb.gen(1).u.q (port out)`), `--hang-report`, `--activity` and `--profile` (text and JSON), coverage reports, rows and exclusions, wave notes, the debugger, and JSON output (`--message-format=json`, `--activity=FILE`, `--profile=FILE`, `yavs/hierarchy`, `cov report --exclusions=json`): one form, so what a report prints can be pasted into a console, a filter or an exclusion row.

A process without a label has no path a reader could know, so every text calls it by what it is and where (#992): `tb.clk <= ... (tb.vhd:17)`, `tb.process (tb.vhd:34)`, `tb.assert (tb.vhd:40)`, `tb.check(...) (tb.vhd:41)`, and `tb.b.GUARD <= ... (tb.vhd:50)` for the guard process of a block. Statements of one region on one line that read alike also get a column: `tb.q <= ... (tb.vhd:10:12)`. That is the same text at any `--threads`, in report and assert lines, run-time errors, `W3102`, the source notes of `E2112`, `--print-hierarchy` (the region in the instance form: `tb(a).clk <= ... (tb.vhd:17)`, so such a line holds blanks), `--hang-report`, `--activity` and `--profile` (text and JSON), the console's `waits`, `drivers`, `activity`, `trace`, `where` and `find` tables, and the debugger's Threads, Call Stack and Hierarchy rows. The subprogram and line tables of `--profile` call such a body `process without a label`, with its place beside it. What is looked up or read by a program keeps the internal path with `_pN`, because it must be one token: the path of a variable of such a process (`tb._p0.total`, as `find variables` lists it, for `examine`, `describe` and Watch), `yavs/hierarchy`, wave scopes, VHPI names and `evaluateName`. `'path_name` and `'instance_name` follow the LRM and are not affected.

Colons stay where something other than yavs's own text defines them or nobody reads them: the values of `'path_name` and `'instance_name` (the LRM's form, compared with the reference simulators) and the `{i}` of `SetVhdlAssertFormat` (the same kind of value in a report's own text: `'instance_name` of the region around the process, without the last colon), the VHPI names (`vhpiFullNameP`), the coverage database (a stored key that tells an instance from a package; older databases stay mergeable), and a data breakpoint's `dataId` (an opaque protocol id; the client shows `description`, dotted).

Every place that takes a path takes both forms: `:tb:u:s` and `tb.u.s` are the same in console and debugger commands, `--wave-include` and `--wave-exclude`, wave layout files, `yavs/waveSubscribe`, and the instance globs of coverage exclusion rows (text and YAML).

## 5a. `yavs lint`

```
yavs lint [FILE|DIR ...] [--recursive] [--work=NAME:PATH] [--map=NAME:PATH ...] [--std=STD]
          [--config=FILE] [--waivers=FILE] [--no-file-list] [--list-rules] [--explain=RULE] [-W...]
          [--json=FILE] [--sarif=FILE] [--junit=FILE] [--quality-report=FILE] [--sonarqube=FILE] [--facts=FILE]
```

- Reads sources again with the analysis `yavs analyze` does and writes nothing into any library: no record, no
  index, no cache entry (the analysed-unit cache is read). `--define`, `--psl` and `--relaxed` must be those the
  files were analyzed with, as for `elab`.
- `file_list` in the configuration is a list of paths, directories and globs (`*`, `?`, `**`; `$NAME`, `${NAME}`; hidden entries skipped by a glob, each glob sorted), relative to the working directory, not to the configuration file. Its files are added after `FILE|DIR`; a file named twice is read once, as first named. `--no-file-list` leaves it out. An entry that gives no file, a form that is not a list, and settings for one entry (`{path: {rule: ...}}`, not implemented) are `E0134`, status 2, before anything is read. `--list-rules` and `--explain` do not look at the files.
- Without `FILE|DIR` and without files from `file_list`, every file recorded in `--work`, in the order of the library. With them, the files recorded
  in a mapped library come first in that library's order and into that library; the others follow in the order
  given, into the work library (`work` when `--work` is not given, kept in memory). A directory is its `.vhd` and
  `.vhdl` files, sorted, with its subdirectories under `--recursive`. `std` and `ieee` cannot be mapped (`E0102`).
- `exclude` in the configuration lists files and directories not to check, relative to the configuration file; a file no library records is still read. A run with nothing left to check is status 0.
- A file that fails analysis prints its errors (not its warnings, which are `yavs analyze`'s) and is not checked;
  the others are.
- Exit status: 0 no finding of severity error; 1 a finding of severity error; 2 usage, a bad configuration or
  waiver file (`E0134`: unreadable, not valid YAML, or a bad value of a key it reads; an unknown key is not one), an unwritable report (`E0135`), nothing to check (`E0133`), a file that fails analysis; 3
  internal error.
- Findings are diagnostics with the rule id as their code on standard error, in `--message-format`; `-W` takes
  rule ids. Report files are written whatever the findings. A waived finding is in no report.
- Configuration: `--config`, else `yavs-lint.yaml` in the working directory or an ancestor. Waivers: `--waivers`,
  else `yavs-lint-waivers.yaml` next to the configuration or in the working directory or an ancestor. Not a contract
  with test runners; `analyze`, `elab` and `run` never read them.
- `yavs lint` is a static analysis tool, not a style guide or formatter. Every key, rule id or rule group of either file that it does not read, at any level, is error `E0134` (status 2, nothing is checked), whatever `-W` says; no setting of another tool is accepted. Every such name of a file is listed, one line each with its line number, in one run, with a suggestion through the shared helper when one is close. A misspelled `files` or `lines` in a waiver would widen it, and an unknown rule would waive nothing, so they are errors like the rest. A bad value of a known key, a duplicate key and a YAML error are `E0134` too. `--list-rules` reads both files first and fails the same way. Aliases in a key it reads are a bad form. Settings for one `file_list` entry stay `E0134`.
- Every YAML file yavs reads (`yavs-lint.yaml`, `yavs-lint-waivers.yaml`, the YAML exclusion file of `yavs cov report --exclude`, the file of `--config`) is checked against one declared schema of keys, value types and nesting before it is used: an unknown key, a value of the wrong type and a missing required key are errors (`E0134`, `E0112`, `E0138`), every problem of a file is listed with its line in one run, each unknown name with a suggestion through the shared helper. A syntax error or a duplicate key stops at the first. A key with no value is an empty list where a list is expected. `docs/lint.rst` and `docs/coverage.md` hold the key tables.
- A waiver's `rule`, `files` and `files_exclude` are one word or a list. It applies to a file `files` matches (no `files`: every file) and `files_exclude` does not; `files_exclude` wins. A pattern is relative to the waiver file and covers a file or a directory above it, like `exclude`; it also matches the path as reported, and a bare name that file anywhere. A pattern that covers no file of the run and no VHDL file under the waiver file is warning `W0132`.

## 6. Waves and the runner's debug option

yavs has no graphical debugger of its own and no `yavs run --gui` (removed 2026-10-29, #871; the
option is now a usage error like any unknown option). Debug with any DAP client (`yavs run --dap`,
section 5, for example VS Code with the extension in `editors/vscode/`). A test runner reaches a
client with `yavs run --dap=0 ...`: `--dap=0` makes yavs pick a free port, and its first line on
stderr is then exactly `yavs: DAP listening on 127.0.0.1:<port>` (section 5); `--dap=PORT` with a
fixed port prints the same line.

The runner package `vunit-yavs` (`python/vunit-yavs`) today: `-g/--gui` is refused at the start
(there is nothing to open the run in).
`--yavs-debug[=PORT]` runs `yavs run --dap=PORT @FILE` (default 4711, the editor extension's
attach port; `0` a free one), reads the `DAP listening` line and prints the address on the
terminal, since the runner shows a test's output only when it fails. It runs one simulation:
`--yavs-debug` with `--elaborate` or `-p` above 1 is refused at the start, and a second
simulation of the run is refused before yavs starts. `elab` never gets `--dap`.
Every other run (not `--yavs-interactive`, not `elab`) also gets `--exit-on-stdin-close`: the test runner keeps a pipe to the child's standard input open and never writes to it, so when the runner ends, even killed with SIGKILL, the pipe closes and the child ends (below).
`--yavs-interactive` runs `yavs run --interactive @FILE` on the terminal, with the same limits, and serves the console hook (section "Console hook"); `--yavs-debug` serves it too.
`--yavs-surfer[=HOST:PORT]` adds `--wave=<test output>/yavs/wave.fst --surfer[=HOST:PORT]`, one simulation like the others.

`--source-map=COMPILED=ORIGINAL` (repeatable) in a debug session shows frames and takes breakpoints in `ORIGINAL` for the statements of the file compiled from `COMPILED`; lines must correspond. Diagnostics keep the compiled path. `vunit-yavs` passes it for preprocessed files under `--yavs-debug` and `--yavs-interactive`. `--dap=PORT` does not take a connection closed before it has sent anything for its session, so a client may look whether the port is open.

### Surfer: `--surfer`

`--surfer` starts `surfer --wcp-initiate PORT` from PATH (its stdio to null; yavs listens on `127.0.0.1:0` and waits up to 30 s), `--surfer=HOST:PORT` connects to a Surfer whose WCP server listens there. Surfer 0.6.0 or later. Strictly opt-in: no other flag, variable or file starts or connects. Without `--wave` the file is `<top>.fst` in the parent of the work library directory, kept after the run, its path a note on stderr (`--wave=FILE` wins; a file that cannot be written is `E0801`, with no fallback). Shown at the start: signals of the top scope with records and arrays one level down, or with `--wave-include` every recorded one; at most 200. Refresh: `--surfer-refresh=DURATION` (above zero), then `YAVS_WAVE_FLUSH`, then 1 s; without `--surfer` nothing changes. End: reload, then `zoom_to_fit`; Surfer stays open. Stops in a debug session, before `stopped`: sync (the file reaches the stop time with the signal values a stopped process reads; when nothing changed at that time, a change that repeats one signal's value marks it; what changes later in the time step follows at the same time, and the last value of a time counts; variables of the stopped process and shared ones too; at a stop inside a process those of other processes follow when the time step ends, at a stop between cycles all are read), reload, `set_cursor` and `set_viewport_to` in fs, a marker at a breakpoint, `set_item_color` on the stopped process's signals. `goto_declaration` from Surfer: `NAME: FILE:LINE` as `output` and the event `yavs/showSource` `{path, line}`. `restart` reloads the new file. `E0130`: cannot start, connect, greet or load, before the simulation, exit 2. `W0131`: the connection ended during the run, or Surfer did not answer at the end, printed after the status is fixed; never promoted by `-W error`. All WCP traffic is on a thread of its own, and the simulation never waits for it. A console's `wave add` waits for its answer at most `YAVS_SURFER_WAIT` plus 1 s (default 5 s plus 1 s), `wave save` twice that; a Surfer already known not to answer gets none of that wait. Surfer not answering for that time, its answering again, a refused command, a load that did not finish in six such waits and the end of the connection are each told when they happen: an `output` event of category `stderr` in a session, stderr otherwise. Only the end of the connection ends the following: while answers are owed nothing new is sent except one `get_item_list` per `wave add`, later news replaces earlier (reload, cursor, colours, markers), and late answers are matched to their commands in order. At the end a Surfer that does not answer costs at most one more wait: `reload` is sent, the signal list is not saved and `zoom_to_fit` is skipped.

### Console: `--interactive` and `--do`

`yavs run --interactive` serves the run with a console on the terminal; `--do=COMMANDS` (repeatable, in order) runs
commands first, and without `--interactive` the process then exits. Both conflict with `--dap`. They are a debug
session like `--dap`: interpreter, one thread (`W0109`), no `--cov` (`E0199`) or `--profile` (`E0125`).
The session stops at time 0 before any process has run, and stays after the end of the run until `quit`.

- Commands: `run [TIME | until COND]`, `step [delta | timestep]`, `next`, `stop`, `examine`/`print`, `force`, `deposit`, `release`, `where`, `break`, `restart`/`reset`, `source`, `help [COMMAND]` (the forms and meaning of one command, from the same table), `quit [STATUS]`/`exit [STATUS]`, and the debugger tools of the user guide's command reference (`docs/console-reference.rst`, which lists every command and subcommand and is checked against the table). `;` separates; `#` at the start of a command and `--` start a comment (not a `--` after a space and before a letter inside a command: a flag of a runner's command). Tab at a terminal prompt is the DAP `completions` request: the first word offers commands only, and after a command what it takes (table `TAKES`, `crates/yavs-dap/src/console.rs`). For a client that is not the console, several targets start with one that is their common prefix, as an editor's list takes its first item on Tab. A client that sends `supportsANSIStyling` in `initialize` gets `help` as an output event with bold command names and an empty result; piped output never has them.
- Standard output: reports, textio output, results of commands (a `--do="run; quit"` session prints what a plain run prints, with the same exit status). Standard error: notices of stops and of the end, errors of commands, diagnostics.
- Exit status: the `quit` argument; else the run's (section 5) when it has ended, else 0. A `--do` command that fails without `--interactive` ends the session with 1. End of input is `quit`.
- A failed command does not end a session with `--interactive`. No prompt is printed when standard input is not a terminal.
- Ctrl-C: under `--interactive` every SIGINT pauses a running simulation (no second-signal exit); with a pipe as standard input, a SIGINT while nothing runs exits 130. SIGTERM keeps its default action. Without `--interactive` SIGINT keeps its default action too, except that a run with a compiled tier (`--tier=auto`) ends on its first Ctrl-C or SIGTERM through yavs's own exit with status 130, also on a plain terminal, after the compilation cache's lists of compiled functions and routes are stored (user, 2026-11-13).
- `restart` rebuilds the simulation from the same elaboration: package and process state, files and `--wave` start over, breakpoints stay. With `--vhpi` or `--load` it is an error, as the library's state cannot be reset.

DAP: `evaluate` with `context: "repl"` takes the same lines when the first word is a command (a plain name stays an evaluation); other contexts are unchanged. The response carries the result text; a command that resumes answers first and sends `continued`, and the next results come as `output` events (`console`). `stop` is taken while the simulation runs. `restart` ends in a `stopped` event with reason `entry`. Over TCP `initialize` announces `supportsRestartRequest`: the `restart` request starts the same design again in the same run, also while it runs; the run goes on to the first breakpoint (`arguments.stopOnEntry`: a stop at entry); with `--vhpi` or `--load` an `output` event of category `important` gives the reason, the answer is an ordinary one and the session goes on (VS Code takes a failed `restart` for the end of the session). `output` events of reports and `OUTPUT` text come as a process makes them. The `threads` response and a process's stack frame carry a label for showing (`tb.stim`, or `tb.clk <= ... (tb.vhd:17)` for a process without a label); paths for lookups are in `yavs/hierarchy`. A plain `--dap` session still ends with the run, except while a test runner's command waits for that run.

### Console hook

A test runner registers its own commands with `YAVS_CONSOLE_HOOK=<port>:<secret>` in the environment of `yavs run --interactive`, `--do` or `--dap`. yavs connects to `127.0.0.1:<port>` before the session starts; anything wrong (a bad value, no connection, no answer in 30 s, a name that is a yavs command) is `E0703`, exit 2, and the secret is not printed. The test runner listens; every message is one line of JSON.

| Direction | Message | Meaning |
|---|---|---|
| yavs to runner | `{"hello":1,"secret":S}` | First line of the connection |
| runner to yavs | `{"commands":[{"name":N,"help":H},...]}` | First answer; a name without spaces. An empty help keeps it out of `help` |
| yavs to runner | `{"call":ID,"name":N,"args":TEXT}` | The command was given; the rest of its line is `args` |
| runner to yavs | `{"print":TEXT}` | Show `TEXT` as it is (add the newline) on the console, or in `output` events under `--dap` |
| runner to yavs | `{"exec":K,"line":TEXT}` | Run a console line. The answer comes when it is done: a run at the next stop or at the end of the simulation |
| runner to yavs | `{"reload":K,"args":[FLAGS]}` | Analyse nothing, elaborate again from the libraries with the run's command line plus `FLAGS`, and go to time 0 with breakpoints kept. On failure the old design stays loaded and its state is untouched. Refused with `--vhpi` or `--load` |
| yavs to runner | `{"result":K,"ok":B,"text":T,"error":E}` | Answer to `exec` and `reload` |
| runner to yavs | `{"done":ID,"result":TEXT}` or `{"done":ID,"result":"","error":E}` | The call ended. A non-empty `result` is shown like a result of a command; `error` fails the command |

While a call is under way, the simulation stays at its end if it gets there. If the connection closes, a waiting command fails with an error and the session goes on. The hook's commands appear in `help`, never in `yavs run --help`.

## 7. Environment

The test runner sets no yavs environment variables (only `YAVS_TOKENS` under `--yavs-jobserver`, section 5). `YAVS_STDLIB` overrides the std/ieee
tree (section 3); `YAVS_STD_CACHE` names the directory of the shared std/ieee units, or `off`; `--std-cache` overrides it (section 3). Simulator hooks from the test runner packages
can add `elab_flags`, `run_flags` and run environment; the test runner applies
`elab_flags` to both `run` and `elab`, and `run_flags` only to `run`. `YAVS_HUGE_HEAP=0` stops yavs
from asking for transparent huge pages on its heap, at start and as the heap grows (Linux with glibc
and the system's transparent huge pages in `madvise` mode only; on any other system it does nothing). `--wave` ends a waveform block by size only, so the
file's bytes depend on the recorded changes alone; `YAVS_WAVE_FLUSH=5s` (or `500ms`) also ends a
block after that much run time, so a killed run leaves more of the file (the bytes then depend on
the run's speed); `never` is the default; any other value is error E0801.

## 8. Not covered yet

Nothing at present.
