# yavs requirements from an external test runner

> Status note (2026-10-28, checked against the CLI): this is the original request, kept as written.
> Section 1 (several files per call, `@FILE`, stop at the first failing file, the lock held for the
> whole call) and section 2 (library locking) are implemented and described in `docs/cli-contract.md`
> sections 3 and 4. The per-file report `yavs: analyzed FILE` (and its JSON object) is not
> implemented: a successful `analyze` prints nothing, and a failing call of several files prints the
> failing file and the files recorded before it. Section 3 (`std.reflection`) is no longer
> pending: yavs ships it in its 2019 `std` (`docs/decisions.md` "Reflection"), so the sentence
> "Reflection is not in the yavs plan yet" below is out of date.

The test runner's yavs support sets these hooks (they come from three open feature requests of the runner):

| Hook | Value | From feature | Needs from yavs |
|---|---|---|---|
| `supports_concurrent_compiles_into_library` | True | `--parallel-compile` | section 2 (locking) |
| `supports_batch_compile` | True | `--batch-compile` | section 1 (multi-file analyze) |
| `batch_file_list_arguments` | `("@{}",)` | `--batch-compile` | section 1 (`@FILE`) |
| `supports_vhdl_reflection()` | True (a target) | `record_dict_pkg` | section 3 (`std.reflection`) |

A test double of yavs implements sections 1 and 2 and passes end to end
with `--parallel-compile -p 8`, `--batch-compile` and both.

## 1. CLI contract: `yavs analyze FILE...` and `@FILE`

Change section 4 of `docs/cli-contract.md` from "exactly one file per call" to:

```
yavs analyze --std=FILE_STD --work=NAME:PATH [--map=NAME:PATH ...] [--color=always]
             [yavs.global_flags] [yavs.a_flags] FILE [FILE ...]
yavs analyze <same options> @FILELIST
```

- **Several files per call.** All files of a call go into the one `--work` library with the same
  options and the same `--std`. The test runner batches only VHDL files whose commands match apart from the
  file name, so files with another standard or other flags arrive in a separate call.
- **`@FILELIST`.** The test runner passes the files of a batch as `@FILE`, written next to its output path
  (`batch_*.txt`) and deleted after the call. This is the existing response file rule (UTF-8, one
  argument per line, taken verbatim, `\n` or `\r\n`, no recursion), with the file names as the
  positionals. The options stay on the command line, so `@FILE` comes last. The test runner writes the file in
  text mode, so it uses `\r\n` on Windows; yavs already accepts that. A file name containing a line
  break cannot be listed: that batch fails, and the test runner then compiles its files one at a time.
- **Order.** Files are analyzed in the order given, which is the test runner's compile order. A later file of
  the batch may use units of an earlier file of the same batch (same library), so each file's units
  must be visible to the next file before the call ends.
- **Stop at the first failing file.** Analysis stops at the first file with errors, and the call
  exits 1. Files after it are not analyzed. Errors inside one file may all be reported (section 0).
  Exit 2 (usage) and 3 (internal) keep their meaning for the whole call.
- **Report which files were analyzed.** yavs prints one line per file it recorded, in order, for example
  `yavs: analyzed FILE` on stdout, and under `--message-format=json` one object
  `{"level": "note", "code": "analyzed", "file": "FILE"}` per file. Today the test runner compiles the
  whole failed batch again one file at a time, so it only needs the exit code. With the report, a later
  test runner change could mark the analyzed files as compiled and retry only from the failing one.
- **What is recorded on failure.** Files analyzed before the failing one stay recorded. Nothing of the
  failing file or of later files is recorded. Analyzing a file again replaces its units, so the test runner's
  file-by-file retry of a failed batch is safe.
- **Lock for the whole call.** The exclusive `_YAVS_LIB` lock of the work library is taken once and
  held for the whole batch, not per file, so no other `analyze` into that library interleaves with the
  batch. The manifest may be committed once at the end or after each file, always as temp file plus
  atomic rename.
- No command-length limit applies: with `@FILE` the test runner lists any number of files
  (the runner's batch command-length limit applies only to compilers without a file list).

## 2. Library locking (plan rule 14) under `--parallel-compile`

With `supports_concurrent_compiles_into_library = True`, the test runner applies no library rule of its own for yavs.
Up to `-p` analyzes run at once, into the same library and into libraries other running analyzes read
(almost every file reads `vunit_lib` while `vunit_lib` files are still being analyzed). yavs must
therefore guarantee:

- **Exclusive lock for writers.** `analyze` holds the exclusive lock on `<work>/_YAVS_LIB` while it
  reads, changes and writes the manifest. It reads the manifest again after taking the lock and merges
  its own units into it, never into a copy read before the lock: a lost update drops units silently.
  The double checks that every file analyzed into `vunit_lib` is in the manifest exactly once.
- **Atomic replacement.** The manifest and every unit file are written to a temp file and renamed, so a
  reader without the lock sees either the old or the new version, never a partial file.
- **Readers of other libraries.** An analyze reading a library that another process is analyzing into
  must not fail ("library not found" or a half-written manifest). It either takes the shared lock
  briefly while reading that manifest or relies on the atomic rename.
- **No deadlock.** VHDL allows libraries that read each other (lib a uses lib b, and lib b uses lib a),
  so two analyzes can each hold the exclusive lock of their own library and need the other one. yavs
  must never block on another library's lock while it holds its own exclusive lock. It reads other
  libraries lock-free, using the atomic rename, or takes their shared locks before its exclusive lock
  and releases them before analysis. If it takes several locks at once, it takes them in one global
  order, such as the canonical path.
- **Shared lock for `run`/`elab`**, only while reading the manifest and its source list, as in the
  contract. A long simulation never blocks an analyze.
- **Read-only libraries never fail.** If the lock file cannot be opened for writing, yavs opens it
  read-only and takes the shared lock. If it cannot be opened at all, yavs skips locking.
- **Lock waits are not errors.** A wait longer than a few seconds prints a note naming the library and
  does not change the exit code. The test runner shows the output of a passed compile, so the note is harmless.
- **Windows:** `LockFileEx` through `std::fs::File::lock`. The temp file plus rename must use
  `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING` (Rust `std::fs::rename` does this), and must survive
  a reader holding the old manifest open. If not, retry on a sharing violation.

Behaviour of the test double that a test relies on: two analyzes into one library that interleave without the lock fail
with exit 3 (`FAKE_YAVS_NO_LOCK=1` shows it). With the lock they never do.

## 3. `std.reflection` for `tb_record_dict_pkg` (VHDL-2019)

`supports_vhdl_reflection()` is True for yavs as a **target**, not a claim. Reflection is not in the
yavs plan yet. Until it lands, the runner's `add_record_dict()` accepts yavs, but `record_dict_pkg.vhd` fails to
analyze (`use std.reflection.all`), so the runner's `record_dict` example fails on yavs. The test runner
acceptance test `test_record_dict_vhdl_2019` stays skipped for yavs (it runs for the reference simulators
only). Enable it for yavs when reflection lands.

What the runner's `record_dict_pkg.vhd` and its testbench use:

**Package `std.reflection`** (LRM 2019 16.11), with `use std.reflection.all`:

- Types: `value_mirror`, `subtype_mirror`, `integer_value_mirror`, `floating_value_mirror`,
  `physical_value_mirror`, `enumeration_value_mirror`, `array_value_mirror`, `array_subtype_mirror`,
  `record_value_mirror`, `record_subtype_mirror`, and the enumeration `value_mirror_class` with
  `class_integer`, `class_floating`, `class_physical`, `class_enumeration`, `class_array`,
  `class_record` (other classes go to `when others`).
- `value_mirror`: `get_value_class`, `get_subtype_mirror`, `to_integer`, `to_floating`,
  `to_physical`, `to_enumeration`, `to_array`, `to_record`.
- `subtype_mirror`: `simple_name`. It is compared after `upper()`, as one of `TIME`, `BOOLEAN`,
  `BIT`, `STD_ULOGIC`/`STD_LOGIC`, `CHARACTER`, `STRING`, `UNSIGNED`/`UNRESOLVED_UNSIGNED`,
  `SIGNED`/`UNRESOLVED_SIGNED` and `STD_ULOGIC_VECTOR`/`STD_LOGIC_VECTOR`. Either the subtype name or
  the base type name is accepted, so the simple name of a resolved subtype such as `std_logic` or
  ieee-2019 `unsigned` may be either one.
- `integer_value_mirror.value`, `floating_value_mirror.value`.
- `physical_value_mirror.value`, multiplied by `std.env.resolution_limit` to get a `time`. The
  value must be in units of the resolution limit, as on a commercial simulator (the primary reference simulator returns fs). yavs must document
  which one it is, and the package may need an `` `if TOOL_NAME = "yavs" `` branch otherwise.
- `enumeration_value_mirror`: `pos`, and `image` for user enumerations (`"busy"`).
- `array_value_mirror`: `get_subtype_mirror` (to `array_subtype_mirror`) and `get(index)` with the
  LRM index of a one-dimensional array.
- `array_subtype_mirror`: `left`, `ascending`, `length`. For both `downto` and `to` arrays,
  `get(left + i)` (ascending) and `get(left - i)` (descending) must return element `i` from the left.
  This is the branch for other simulators.
- `record_value_mirror`: `get_subtype_mirror` (to `record_subtype_mirror`) and `get(idx)` for an element
  index `0 .. length - 1`.
- `record_subtype_mirror`: `length` and `element_name(idx)`. Names may be returned in any case; the
  package lower-cases them, and `MixedCase` must come back as `mixedcase`.

**Attribute `'reflect`** (LRM 2019 16.2): `value'reflect` on a parameter whose type is a **formal
generic type** of a generic function (`to_dict generic (type record_t) parameter (value : record_t)`)
returns the mirror of the actual type: the record type of each instance, or `integer` for the
instance used to test the "not a record" failure.

**Other 2019 language features in the same files:**

- Generic subprograms with an interface type and the `parameter` reserved word in the
  specification, instantiated five times in one architecture under the same name
  (`function to_dict is new to_dict generic map (record_t => ...)`), resolved by overloading on the
  record type.
- Mirror objects as `variable ... : inout value_mirror` parameters (protected type or access
  semantics), passed between subprograms and assigned to variables of other mirror types.
- Conditional analysis: `` `if TOOL_NAME = "sim1" then ... `else ... `end if ``. yavs defines
  `TOOL_NAME` as `yavs` (decisions.md), so it takes the LRM branch.
- `std.env.resolution_limit` (2019).
- 2019 call paths (`check_only_log` compares the log location), which yavs already reports through
  `supports_vhdl_call_paths`.

Test that defines "done": the runner's `run.py` with yavs as the simulator and VHDL-2019 in the runner's `record_dict` example directory, 6 of 6 passing (scalars, arrays including ascending ranges and signed/unsigned,
nested records, lower-case keys, and the two failure messages).
