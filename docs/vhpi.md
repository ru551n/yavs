# VHPI

VHPI is the standard C interface (IEEE 1076-2019) that lets a plugin look into a running VHDL
simulation, read and write signals, and react to simulation events. yavs implements it: plugins
written in C, C++ or Rust can inspect the design hierarchy, read and drive values, register
callbacks and supply foreign models. Most of the standard is there; the table below says what is missing, and the
open work is listed in the epic [#143](https://github.com/ru551n/yavs/issues/143) (the issues #978, #979 and, for save and restart, #832 were folded into it on 2026-11-02).
For calling a plain C function from VHDL without a plugin, see "Calling C from VHDL" below.

```{contents} On this page
:local:
```

## At a glance

| Area | Supported | Not supported yet |
|---|---|---|
| Loading | `--vhpi=LIB[:ENTRY]`, several plugins, `+args` | |
| Connectivity | drivers, contributors, loads, the simulated net, basic signals, pending transactions, resolution, signal kind, whether a signal is forced | processes that read a signal, as loads |
| Hierarchy | instances, blocks, generates, processes, packages, generics, ports, signals, constants, variables, record fields, array elements, user attributes, aliases, files, implicit signals, components, subprograms, libraries, use clauses, equivalent processes of concurrent statements with their sensitivity, sequential statements | expressions, variables of subprograms, stack frames, `vhpiParent` of a sequential statement, configuration declarations |
| Lookup | by name, by index, iterators, parent/prefix handles | |
| Reading values | all formats, from callbacks, whole records and arrays of records as raw data | protected objects (they have no value), an alias of part of an object, `vhpiPtrVal` of a variable of an access type |
| Writing values | signals and ports (deposit or force with propagation), variables, driver transactions | forcing variables, constants, generics, non-propagating modes on signals |
| Callbacks | start/end of simulation and initialization, time step, delta cycle, process and postponed phases, after delay, time-out, value change, sensitivity on a signal, transaction, force, release, start and end of the tool and of elaboration, errors, Ctrl-C, debugger stops, start and end of a reset | statements, subprogram calls, suspend and resume of a process, analysis (`yavs run` does not analyse), quiescence, save and restart |
| Foreign models | functions, procedures, architectures, library and application hooks | file parameters |
| Misc calls | `vhpi_control` stop, finish and reset, `vhpiPhaseP`, `vhpi_format_value`, `vhpi_get_next_time` (in the read-only phase), `vhpi_protected_call` | `vhpi_get_data`, `vhpi_put_data`: yavs cannot save a simulation (exported so plugins link; they return 0 with an error) |

## A first plugin

The plugin below prints the value of the signal `q` of the top level when the simulation ends.
Build it with the header from the source tree,
[`vhpi_user.h`](https://github.com/ru551n/yavs/blob/main/crates/yavs-vhpi/include/vhpi_user.h) (the release
archives do not contain it). On Linux and macOS the `vhpi_*` functions stay unresolved at link time
and bind to the `yavs` executable when the plugin loads:

```c
#include <stdio.h>
#include "vhpi_user.h"

static void at_end(const vhpiCbDataT *cb) {
  (void)cb;
  vhpiHandleT root = vhpi_handle(vhpiRootInst, NULL);
  vhpiHandleT sig = vhpi_handle_by_name("q", root);
  char buf[16];
  vhpiValueT v = { .format = vhpiBinStrVal, .bufSize = sizeof buf,
                   .value.str = (vhpiCharT *)buf };
  if (sig && vhpi_get_value(sig, &v) == 0)
    printf("%s q = %s\n", (const char *)vhpi_get_str(vhpiFullNameP, root), buf);
}

static void startup(void) {
  vhpiCbDataT cb = { .reason = vhpiCbEndOfSimulation, .cb_rtn = at_end };
  vhpi_register_cb(&cb, 0);
}

void (*vhpi_startup_routines[])(void) = { startup, NULL };
```

```
gcc -shared -fPIC -I path/to/yavs/crates/yavs-vhpi/include hello_vhpi.c -o hello_vhpi.so
yavs analyze --work=lib:out/lib dut.vhd tb.vhd
yavs run --work=lib:out/lib --vhpi=./hello_vhpi.so tb
```

On macOS the linker refuses unresolved symbols in a library unless told otherwise, so build the
plugin with `cc -shared -fPIC -undefined dynamic_lookup -I path/to/yavs/crates/yavs-vhpi/include
hello_vhpi.c -o hello_vhpi.dylib`.

For a top level `tb` with an unsigned signal `q` the run ends with a line such as `:TB q = 0110`
(the full name of the root instance is upper case; the plugin reads names case-insensitively).
Without `--vhpi` the plugin does nothing; it is not part of the design.


## Resetting a simulation

`vhpi_control(vhpiReset)` returns 0, the current cycle finishes, then the simulation is built again
and starts from time 0 with the initial values of the design:

1. `vhpiCbStartOfReset` runs, with the old simulation still readable. `vhpiCbEndOfSimulation` does
   not run. `vhpiPhaseP` is `vhpiResetPhase`.
2. `vhpiCbEndOfReset` runs at time 0 with the new simulation, then the initialization callbacks as
   in any run.

What stays: the plugin and its own state, handles, and every callback that has not matured. A
one-shot callback that ran is gone, so register what must run again in `vhpiCbEndOfReset`. An
after-delay callback counts its delay again from time 0. Puts, forces and transactions of the old
run are gone. Foreign models keep their `elabf` and `execf` calls of the first run: they do not run
again.

A reset is refused with an error when a run reports on its whole length (`--wave`, `--cov`,
`--profile`, `--activity`, `--assert-summary`) or is a debug session; `vhpiProvidesReset` in
`vhpiCapabilitiesP` says which it is. Outside a running simulation it is an error too.

## Loading a plugin

```
yavs run --vhpi=LIB[:ENTRY] [--vhpi=...] TOP
```

- Plugins load in command-line order, after elaboration and before the simulation starts.
- `vhpiPhaseP` (of the tool handle or NULL) follows registration, elaboration, initialization,
  simulation and termination. When all plugins have loaded come `vhpiCbStartOfTool` and
  `vhpiCbStartOfElaboration`, then the foreign models, then `vhpiCbEndOfElaboration`. yavs has
  elaborated the design by then, so its hierarchy can already be read. `vhpiCbEndOfTool` is the
  last call of a run, whichever way it ended.
- `vhpi_control(vhpiReset)` starts the design again from time 0 while the plugin stays loaded, see "Resetting a simulation" above.
- Without `ENTRY`, every routine in the library's NULL-terminated `vhpi_startup_routines` array
  runs. With `ENTRY`, that `void (*)(void)` function runs instead.
- A name with a directory part must name an existing file; a bare name goes through the operating
  system's library search path.
- A plugin that cannot be loaded is error `E0901`, one with no entry `E0902`; both end the run with
  exit status 2 before time 0.
- Linux and macOS: a plugin leaves the `vhpi_*` symbols unresolved (on macOS, link it with
  `-undefined dynamic_lookup`); they bind to the `yavs` executable. Windows:
  `yavs.exe` exports the same set; a plugin brings its own import library or resolves the names at
  run time.
- Plugin calls are serialized, so a plugin needs no locking whatever `--threads` says. VHPI times
  count in units of `--resolution`.
- `vhpi_get(1100, tool)` or `vhpi_get(1100, NULL)` is the seed of `--shuffle` (`--shuffle` alone is
  seed 1). 1100 is the extension property `vhpiRandomSeedP`: it is not in `vhpi_user.h`, so a plugin
  defines it itself. A run without `--shuffle` has no seed, and a seed above 2147483647 does not fit
  `vhpiIntT`: both return -1 with a message for `vhpi_check_error`, and no warning. A seed is never
  negative, so -1 is never a real one.
- A call yavs does not implement fails with a "not implemented" message that the plugin reads with
  `vhpi_check_error` (warning `W0904`); a misused call is warning `W0903`.

## Plusargs

Arguments of the form `+NAME` or `+NAME=VALUE` after the top level are passed to plugins, in order:
`yavs run --work=lib:out/lib --vhpi=./plugin.so tb +seed=7 +verbose`. A plugin reads them with
`vhpi_get(vhpiArgcP, tool)` and the `vhpiArgvs` iterator; the first argument is the program name
`yavs`. VHDL code cannot see them.

## Calling C from VHDL

A subprogram whose `foreign` attribute is `"VHPIDIRECT symbol"` calls the C function `symbol`.
`--load=LIB` loads a shared library (repeatable); symbols are looked up in the libraries in order,
then in the `yavs` executable. A symbol that is defined nowhere is warning `W0108` before the
simulation starts and error `E0107` when the subprogram is called.

```vhdl
function add1 (x : integer) return integer;
attribute foreign of add1 : function is "VHPIDIRECT add1";
function add1 (x : integer) return integer is begin return 0; end; -- placeholder, never runs
```

```c
int32_t add1(int32_t x) { return x + 1; }
```

```
yavs run --work=lib:out/lib --load=./libadd.so tb
```

Types map to C as follows: an integer subtype to the smallest of `int8_t` to `int64_t` that holds
its range (`integer` is `int32_t`), an enumeration to the smallest unsigned type that holds its
literals (`boolean`, `bit`, `character` and `std_logic` are `uint8_t`), `real` to `double`, a
physical type to `int64_t`. Scalar `out` and `inout` parameters are passed as pointers. An array is
a pointer to its elements from left to right (a `std_logic` element is one byte, `'0'` is 2 and
`'1'` is 3), and an unconstrained array parameter is followed by one `int64_t` length per
dimension. A record of scalars is a pointer to a C struct, an access value a pointer to the
designated object.

### The fat-pointer layout

Some C code expects another argument layout. `--load=LIB:args=fat-pointer` selects it for the one
library; every other library keeps the layout above, which `--load=LIB:args=length` also names. The
layout belongs to the library that defines the symbol, so one run can mix both.

An unconstrained array is then one pointer, to a struct that holds the data pointer and a pointer to
its bounds. The bounds are one struct per dimension, in order and naturally aligned, whose `left` and
`right` have the C type of that index type; `dir` is 0 for `to` and 1 for `downto`:

```c
struct bounds { int32_t left, right; uint8_t dir; uint32_t len; }; /* an index of 32 bits */
struct fat_pointer { void *data; struct bounds *bounds; };

int32_t str_len(struct fat_pointer *s) { return s->bounds->len; }
```

```vhdl
function str_len (s : string) return integer;
attribute foreign of str_len : function is "VHPIDIRECT str_len";
```

```
yavs run --work=lib:out/lib --load=./libhelpers.so:args=fat-pointer tb
```

| VHDL | `length` (default) | `fat-pointer` |
|---|---|---|
| Integer type or subtype, by value | smallest of `int8_t` to `int64_t` that holds the subtype | `int32_t`, or `int64_t` when the base type does not fit 32 bits |
| Enumeration | smallest unsigned type that holds its literals | `uint8_t`, or `uint32_t` above 256 literals |
| `real`, physical type | `double`, `int64_t` | the same |
| Scalar `out` or `inout` | pointer to the C type above | the same |
| Constrained array | pointer to its elements, left to right | the same, in the element types above |
| Unconstrained array | pointer to its elements, then one `int64_t` length per dimension | pointer to the struct shown above |
| Array of `std_logic`, `bit`, `boolean`, `character` | one byte per element | the same |
| Record of scalars | pointer to a naturally aligned C struct | the same, in the field types above |
| Access value | pointer to the designated object | not supported: error `E0107` when the run starts |
| Function result | scalar by value | the same |
| Record or array result, access result | not supported: error `E2001` | the same |

A record with composite fields and an array of composite elements are errors at the call in both
layouts. An unknown layout name is error `E0136`.

The libraries load before elaboration, so a foreign function can give a signal or constant its
initial value (`signal id : integer := open_image(name);`). `yavs elab` takes `--load` too, so a
design like that elaborates alone (and under VUnit's `--elaborate`). A call during elaboration can run again when the simulation is built if
elaboration cannot hand its package state over (coverage of statements, for example): keep such
functions free of effects that cannot be repeated.

## Full detail

The per-area audit of the standard with the open issues, and call-by-call behaviour, are in
[`crates/yavs-vhpi/STANDARD.md`](https://github.com/ru551n/yavs/blob/main/crates/yavs-vhpi/STANDARD.md).

## See also

- [Command-line reference](cli-reference.rst): `--vhpi` and `--load`.
- [Error codes](error-codes.rst): `E0901`, `W0903` and the others.
