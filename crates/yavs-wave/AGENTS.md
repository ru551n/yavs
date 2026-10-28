# AGENTS.md: yavs-wave

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-wave`.
The writer choice and the FST mapping are in `docs/decisions.md` "Waveform writer: wavefst".

## 1. Purpose and boundaries

- Waveform recording sink: a `Hierarchy` description (built with `HierarchyBuilder`) plus a
  stream of `(time, SignalId, value bytes)` changes into `WaveWriter`, written as FST through the
  `wavefst` crate; include/exclude globs and an array size cap (`Selection`); SIGINT/SIGTERM flag
  (`interrupt::install`) and the one signal thread of `yavs run` (`interrupt::start`).
- `src/sim.rs` (`record`, `Recorder`): builds the `Hierarchy` from an elaborated design and
  subscribes to the kernel's value-change bus; the writer sink itself stays kernel-free.
  `yavs run --wave` wires it (E0801-E0803, W0802).
- Depends on: `wavefst` (writer, lz4; no C compiler), `signal-hook` (Unix), `ctrlc` (Windows); dev: `wellen` (read-back),
  `tempfile`. Called by: `yavs` (`run --wave`), `yavs-dap` (live waves, `record_to`), `examples/demo.rs`; `examples/waveq.rs` reads files only.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Re-exports; the value byte layouts per `SigType` (crate docs) |
| `src/model.rs` | `Hierarchy` (a flat depth-first list of `Item`s, names in one string, `SigType`s shared by index; walked with `Hierarchy::items` as `HierItem::{Scope, Signal, Up}`), `HierarchyBuilder` (`signal_shared` takes an `Arc<SigType>` that many signals share), `ScopeKind` (`Record`, `Array`, ...), `SignalId`, `SigType`, `ScalarType`, `EnumType`, `Bounds`, `Dir`, `ObjClass`, `PortDir`, `Skipped` (`elems: None`: a variable whose bounds are known only at run time, `HierarchyBuilder::unknown`) |
| `src/select.rs` | `Selection`: case-insensitive globs (`*` one level, `**` across, `?`), `max_array_elems` |
| `src/writer.rs` | `WaveWriter` (`change`, `finish`, also on drop; one declaration plan per `Hierarchy` type, a slot of two `u32`s per signal), `WaveConfig` (timescale `-15` fs, `flush_interval` off by default, `block_change_limit`), `Error` |
| `src/sim.rs` | `Sink` (where a recorder writes: `WaveWriter`, or the DAP live stream), `record_to` (a recorder into any sink, returns the hierarchy), `Recorder::start` (a recorder subscribed after the start), `record` (with `Option<&Built>` for `--wave-variables`), `Recorder`: elaborated design to `Hierarchy`; kernel changes to `change` calls, final value per time step in entry order (same file at any `--threads`) |
| `src/interrupt.rs` | `start(Handlers)`: one thread for the first Ctrl-C/SIGTERM/SIGHUP (`first`: the kernel ends between cycles, file finished), the second (`at_exit`, exit 1), SIGUSR1, a periodic `tick` (the output flusher of `yavs run`) and a one-shot `after` (the start of the compile thread of `--tier=auto`, 5 ms into the run, #426; a signal does not hasten it); `install(stop)` and `install_with` for `first` and `at_exit` only. Unix: `signal-hook` flag plus a byte on a socket pair (`register_raw` on descriptors that stay open, no copy per signal), safe code; Windows: `ctrlc` and a tick thread (#562) |
| `tests/roundtrip.rs` | Every file read back with wellen |
| `examples/writer_bench.rs` | Writer throughput on synthetic changes (`SCALARS`, `VECTORS`): the per-change cost of the writer alone |
| `examples/demo.rs` | Small end-to-end use |
| `examples/waveq.rs` | Comparer of two waveform files (FST, VCD, GHW through `wellen`) for `tools/waveq` (#781); its unit tests run with `cargo test` (`test = true`) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| A signal's entries are kept per kernel leaf in layers of disjoint ranges: a port collapsed into its actual (yavs-elab `collapse.rs`) shares the actual's elements, a part of a leaf (`SigObj::part`) included; each object is still recorded under its own name | #413: the same waves with ports collapsed or kept apart (`tests/wave.rs` `corpus_waves` compares the `hierarchy` and `force` groups both ways) | `docs/decisions.md` "Elaboration" |
| `wavefst` 0.4.0 for FST, behind our own `WaveWriter` API; no FST code of our own | Enum tables, VHDL var types, reals, header-patching flush | `docs/decisions.md` "Waveform writer: wavefst" |
| std_logic family and bit written as characters with the VHDL data type set; other enums as bit vectors with an FST enum table (original spelling); integers/time two's complement; reals doubles; vector bounds in the name (`data[7:0]`, left first) | Viewers show `U`, `X`, direction kept | same, "FST mapping" |
| Records and arrays are scopes with one signal per sub-element | Rule 11 (same sub-element events as everything else) | `docs/plan.md` rule 11 |
| An integer is written with its subtype's name and the width of its range (`Shape::Scalar::{sub, range}`; none: base name, 64 bits); the chosen generate alternative is the scope `label.alt` (`ScopeKind::Generate::alt`), so `*` in a glob does not span `label.alt` | Viewers show the declared type; #306 | `docs/decisions.md` "Waveform writer: wavefst" |
| Variables (`--wave-variables`, off by default: process variables and non-protected shared variables) are read once per time step, at `NextTimeStep` and `EndOfSim`, through `Hook::with_process`/`InterpProcess::var` and `yavs_interp::shared_var`, read from the raw bytes of the value at the places `yavs_elab::leaf_locs` finds once per variable (no decoded `Val` tree), and declared from `Elaborated::var_shapes` with the signals' `node` walk (records and arrays as scopes); a sub-element is written when it differs from its last written value; a process's variables are read only after `InterpProcess::execs` changed (it ran or had a variable written), and a value is decoded only when it differs from the last raw value, which `InterpProcess::var_is` / `shared_var_is` check in place without copying (a composite in a heap object whose write `Stamp` is unchanged is not even compared; #418) | Variables are not on the event bus; every process is suspended between phases, so any `--threads` gives the same file; #305, #348, #321 | `docs/decisions.md` "Waveform writer: wavefst" |
| A design can have millions of entries (a testbench of verification components with `--wave-variables` has two million), so per-entry state is flat and shared: the `Hierarchy` is one item list plus one name string with the types interned, the recorder keeps an entry as 20 bytes and a variable's paths, places (`DataLeaf`) and last values in one vector each, the writer plans a type once, and the builder builds paths only when the selection has patterns | Peak memory and declare time scale with entries (#418) | `src/model.rs`, `src/sim.rs`, `src/writer.rs` |
| A variable that changed is re-read only where its bytes changed: the raw value is compared with the one before it in 64-byte chunks (a prefix count of the differing chunks), and an entry is read only if its bytes (`DataLeaf::span`) overlap a differing chunk | Large records of arrays change a few entries at a time (#418) | `src/sim.rs` `chunk_counts` |
| `WaveWriter::create` raises `block_change_limit` to twice the number of signals | An FST block opens with the value of every variable, so with two million variables a 250 000-change block is almost all of those values (9 blocks of 40 MB for the initial values alone). Files of up to 125 000 signals are unchanged byte for byte; larger ones differ in block layout only (#418) | `src/writer.rs` |
| With `--wave` (`record`; not `record_to`, the live stream) an entry equal to one already declared (same kernel leaf, offset, length, conversion, interned type and class) is not an entry of its own: its name is declared again with the first one's `SignalId`, which the writer makes an FST alias (with the enum table reference and metadata). A collapsed port, at every level of the hierarchy, costs nothing at run time and a name in the file | Port chains made 3 to 10 entries per value (neorv32: 14.6k names for 3.1k values); #410 | `src/sim.rs` `Design::signal`, `src/writer.rs` `declare_signal` |
| The recorder reads values in place (`Hook::view`/`SimControl::view`, no `Value`), turns a logic vector into positions eight elements at a time from the plane words (`logic_rev`), writes a vector as ASCII bytes (`SignalValue::Bytes`) through a 256-entry table, and a scalar `0`/`1` through `emit_binary_change`; `value_changes` marks the entries of a whole cycle under one lock, a leaf with one covering entry (`State::solo`) without a search and a layer of equal tiles (an array of vectors) by division (`Layer::tiles`); the recorder asks for four phases only (`wants_phase`), the kernel builds no hook for the others (a chain of 1000 stages: +57 percent became +17) | Per change cost: allocations, a lock and a search per range were most of it (#410) | `src/sim.rs`, `src/writer.rs` |
| `wavefst` is built with its `simd` feature (packing of `0`/`1` vectors); value chains are LZ4-compressed (`ChainCompression::Lz4`; the hierarchy block already needs LZ4 in a viewer): files 50 to 60 percent smaller for 0 to 6 percent more instructions with waves | The scalar packing loop was 27 percent of the writer; byte-identical output | `Cargo.toml` |
| Include/exclude globs, size cap for arrays and memories, enum names in metadata | Rule 11 | `docs/plan.md` rule 11 |
| Flushed on the next time advance after `block_change_limit` changes, and after `flush_interval` of wall clock only if `YAVS_WAVE_FLUSH=<duration>` sets one (default off: the bytes depend on the changes only, #666); header patched each time; SIGINT asks the kernel to stop between cycles and the recorder calls `finish` | A killed run is readable | `docs/decisions.md` "Durability" |
| The recorder writes through the `Sink` trait (`change`, `step` at the end of each time step, `finish`); `WaveWriter` is one sink, the DAP server's live stream another (#325) | One hierarchy, selection and per-step value logic for the file and the stream | issue #325, `docs/design/gui.md` 5.1 |
| Waves come in through the event bus as a subscriber (M7) | Plan "Designed for the GUI" | `docs/plan.md` |
| Identifiers keep original spelling in the file; matching is case-insensitive | Rule 8 | `docs/plan.md` rule 8, `src/select.rs` |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Value bytes follow the per-`SigType` layouts in the crate docs (vector: one byte per element,
  the enumeration position). The kernel stores logic as bit-planes and BIT/BOOLEAN array elements
  as std_ulogic codes 2/3: the subscriber must transpose and translate at the boundary
  (`docs/decisions.md` "Interpreter: BIT/BOOLEAN codes").
- `HierarchyBuilder::signal` returns false for a signal the selection or the cap drops; the
  recorder creates a kernel entry only for kept signals, so left-out signals cost nothing.
- Never write FST bytes by hand; keep all `wavefst` use inside `writer.rs` so it can be swapped.
- Tests write into `tempfile` dirs; keep large outputs out of `/tmp` (root rule 20).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-wave          # round trips through wellen
cargo run -p yavs-wave --example demo
```

- No diagnostic codes yet; I/O failures are `writer::Error`. When wired into the CLI, its errors
  need codes (pick a range and add it to the registry in `crates/yavs-diag/src/lib.rs`).

## 6. Coordination

- Open issues (snapshot 2026-10-18; #306 done, `gh issue list -R ru551n/yavs --label crate:wave`): #129 FST
  output as an event-bus subscriber, hierarchy, enum tables, globs and size caps; #116 `--wave`
  wired into `yavs run` (done: `yavs run --wave` exists, `docs/cli-contract.md`). Epic: #145 (M7).
