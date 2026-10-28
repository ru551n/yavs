# VHPI

VHPI is the standard C interface (IEEE 1076-2019) that lets a plugin look into a running VHDL
simulation, read and write signals, and react to simulation events. yavs implements it: plugins
written in C, C++ or Rust can inspect the design hierarchy, read and drive values, register
callbacks and supply foreign models. Support is partial: the table below says what is missing.
For calling a plain C function from VHDL without a plugin, see "Calling C from VHDL" below.

## At a glance

| Area | Supported | Not supported yet |
|---|---|---|
| Loading | `--vhpi=LIB[:ENTRY]`, several plugins, `+args` | |
| Hierarchy | instances, blocks, generates, processes, packages, generics, ports, signals, constants, variables, record fields, array elements, user attributes of declarations | aliases, files, equivalent processes of concurrent statements, statements, variables of subprograms |
| Lookup | by name, by index, iterators, parent/prefix handles | |
| Reading values | all common formats, from callbacks | whole records, protected objects, raw data |
| Writing values | signals and ports (deposit or force with propagation), variables, driver transactions | forcing variables, constants, generics, non-propagating modes on signals |
| Callbacks | start/end of simulation and initialization, time step, delta cycle, process and postponed phases, after delay, time-out, value change, sensitivity on a signal | transactions, force and release, statements, subprogram calls, tool phases, save and restart |
| Foreign models | functions, procedures, architectures, library and application hooks | file parameters |
| Misc calls | `vhpi_control` stop and finish, `vhpi_format_value`, `vhpi_get_next_time` (in the read-only phase) | `vhpi_get_data`, `vhpi_put_data`, `vhpi_protected_call` (exported so plugins link, but they report "not implemented") |

## A first plugin

The plugin below prints the value of the signal `q` of the top level when the simulation ends.
Build it with the header from the source tree,
[`vhpi_user.h`](https://github.com/ru551n/yavs/blob/main/crates/yavs-vhpi/include/vhpi_user.h) (the release
archives do not contain it). On Linux the `vhpi_*` functions stay unresolved at link time and bind
to the `yavs` executable when the plugin loads:

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

For a top level `tb` with an unsigned signal `q` the run ends with a line such as `:TB q = 0110`
(the full name of the root instance is upper case; the plugin reads names case-insensitively).
Without `--vhpi` the plugin does nothing; it is not part of the design.

## Loading a plugin

```
yavs run --vhpi=LIB[:ENTRY] [--vhpi=...] TOP
```

- Plugins load in command-line order, after elaboration and before the simulation starts.
- Without `ENTRY`, every routine in the library's NULL-terminated `vhpi_startup_routines` array
  runs. With `ENTRY`, that `void (*)(void)` function runs instead.
- A name with a directory part must name an existing file; a bare name goes through the operating
  system's library search path.
- A plugin that cannot be loaded is error `E0901`, one with no entry `E0902`; both end the run with
  exit status 2 before time 0.
- Linux: a plugin leaves the `vhpi_*` symbols unresolved; they bind to the `yavs` executable. Windows:
  `yavs.exe` exports the same set; a plugin brings its own import library or resolves the names at
  run time.
- Plugin calls are serialized, so a plugin needs no locking whatever `--threads` says. VHPI times
  count in units of `--resolution`.
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

## Full detail

The per-area audit of the standard with the open issues, and call-by-call behaviour, are in
[`crates/yavs-vhpi/STANDARD.md`](https://github.com/ru551n/yavs/blob/main/crates/yavs-vhpi/STANDARD.md).
