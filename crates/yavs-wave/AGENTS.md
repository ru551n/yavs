# AGENTS.md: yavs-wave

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-wave`.
The writer choice and the FST mapping are in `docs/decisions.md` "Waveform writer: wavefst".

## 1. Purpose and boundaries

- Waveform recording sink: a `Hierarchy` description (built with `HierarchyBuilder`) plus a
  stream of `(time, SignalId, value bytes)` changes into `WaveWriter`, written as FST through the
  `wavefst` crate; include/exclude globs and an array size cap (`Selection`); SIGINT/SIGTERM flag
  (`interrupt::install`).
- `src/sim.rs` (`record`, `Recorder`): builds the `Hierarchy` from an elaborated design and
  subscribes to the kernel's value-change bus; the writer sink itself stays kernel-free.
  `yavs run --wave` wires it (E0801-E0803, W0802).
- Depends on: `wavefst` (writer, lz4; no C compiler), `ctrlc`; dev: `wellen` (read-back),
  `tempfile`. Called by: `yavs` (`run --wave`), `examples/demo.rs`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Re-exports; the value byte layouts per `SigType` (crate docs) |
| `src/model.rs` | `Hierarchy`, `HierarchyBuilder`, `Scope`/`ScopeKind` (`Record`, `Array`, ...), `Signal`, `SignalId`, `SigType`, `ScalarType`, `EnumType`, `Bounds`, `Dir`, `ObjClass`, `PortDir`, `Skipped` |
| `src/select.rs` | `Selection`: case-insensitive globs (`*` one level, `**` across, `?`), `max_array_elems` |
| `src/writer.rs` | `WaveWriter` (`change`, `finish`, also on drop), `WaveConfig` (timescale `-15` fs, `flush_interval` 5 s, `block_change_limit`), `Error` |
| `src/sim.rs` | `record` (with `Option<&VarProbes>`), `Recorder`: elaborated design to `Hierarchy`; kernel changes to `change` calls, final value per time step in entry order (same file at any `--threads`) |
| `src/interrupt.rs` | `install(stop)`: first Ctrl-C/SIGTERM calls `stop` (kernel ends between cycles, file finished); second exits 1 |
| `tests/roundtrip.rs` | Every file read back with wellen |
| `examples/demo.rs` | Small end-to-end use |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| `wavefst` 0.4.0 for FST, behind our own `WaveWriter` API; no FST code of our own | Enum tables, VHDL var types, reals, header-patching flush | `docs/decisions.md` "Waveform writer: wavefst" |
| std_logic family and bit written as characters with the VHDL data type set; other enums as bit vectors with an FST enum table (original spelling); integers/time two's complement; reals doubles; vector bounds in the name (`data[7:0]`, left first) | Viewers show `U`, `X`, direction kept | same, "FST mapping" |
| Records and arrays are scopes with one signal per sub-element | Rule 11 (same sub-element events as everything else) | `docs/plan.md` rule 11 |
| An integer is written with its subtype's name and the width of its range (`Shape::Scalar::{sub, range}`; none: base name, 64 bits); the chosen generate alternative is the scope `label.alt` (`ScopeKind::Generate::alt`), so `*` in a glob does not span `label.alt` | Viewers show the declared type; #306 | `docs/decisions.md` "Waveform writer: wavefst" |
| Process variables (`--wave-variables`, off by default) are sampled once per time step, at `NextTimeStep` and `EndOfSim`, from the bytes the process published when it last suspended (`yavs_interp::ProcVars`, watched only when the selection keeps them); a variable is written when it differs from its last written value | Variables are not on the event bus; the value at a suspension is the value at the end of the step, so any `--threads` gives the same file; #305 | `docs/decisions.md` "Waveform writer: wavefst" |
| Include/exclude globs, size cap for arrays and memories, enum names in metadata | Rule 11 | `docs/plan.md` rule 11 |
| Flushed on the next time advance after `flush_interval` or `block_change_limit`; header patched each time; SIGINT asks the kernel to stop between cycles and the recorder calls `finish` | A killed run is readable | `docs/decisions.md` "Durability" |
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
  wired into `yavs run`. Epic: #145 (M7).
