# AGENTS.md: tools/waveq/ (waveform equivalence)

Read the root `AGENTS.md`, `tools/AGENTS.md` and `tools/realbench/AGENTS.md` first. This file is for an agent that
runs or changes the waveform comparison of yavs against the reference simulators (#781).

## 1. Purpose and boundaries

- Evidence that yavs computes the same signal values as the reference simulators on real designs, signal by signal and
  time step by time step, not only the same test verdict. It measures and classifies; it never changes yavs.
- `run.py` runs each design on yavs (`--tier=interp`, `auto` and `llvm`) and on the references, each writing a waveform
  file, and compares them with the comparer. The comparer is `crates/yavs-wave/examples/waveq.rs` (reads FST, VCD and
  GHW through the `wellen` dev-dependency; built on demand, `--comparer` takes a built one). Python is stdlib only.
- Designs: the h2h RTL designs of `tools/bench/designs` (small generics) and realbench tests (worlds built by
  `tools/realbench/run.py`, imported). CI cannot run the references: results say "run locally on DATE at COMMIT".

## 2. Map

| Path | What |
|---|---|
| `run.py` | Driver: `BIN [--group h2h|real|all] [--designs a,b] [--refs sim1,sim2] [--tiers interp,auto,llvm] [--keep]`, `--report JSON`, `--self-test`; `H2H` and `REAL` (designs, stop times, comparer skip globs), `KNOWN` (explained differences) |
| `RESULTS.md`, `results.json` | The last full run (copied from the cache by hand), the form `docs/conformance.md` reads (section 3) |
| `crates/yavs-wave/examples/waveq.rs` | Comparer: `waveq A B [--json OUT] [--skip GLOB]... [--until FS] [--context N]`, `waveq --list FILE`; `cargo test -p yavs-wave --example waveq` |

Output: `$YAVS_WAVEQ_CACHE` (default `~/.cache/yavs-waveq`): `<time>.json` and `.md` (the slot format), `<time>.full.json`
(every run and pair), `runs/<design>/` (waveforms with `--keep`, comparer JSON per pair).

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Values are compared per time step after its last delta (FST and GHW keep no deltas); a signal is equal when its value agrees at every time either file has a change, up to the stop time (default the later of the two ends, so a missing change at the end is a difference) | What the file formats carry; a cut at the earlier end would hide differences | #781 |
| Names: lower case, `(i)` and `[i]` the same, an element scope `a.[i]` and `a.a[i]` are `a[i]`, a trailing `[msb:lsb]` dropped; a name that still does not match pairs with one that differs by one dropped hierarchy level (a case-generate alternative or a package's library one writer leaves out) | The writers differ only in these spellings (checked on a design with every shape: generates, records, arrays, packages) | #781 |
| Values: 9-value logic as written; enumeration codes through the file's enum table; booleans as names; integers of 32 bits or more as signed decimals; a narrower integer equals a decimal that is its value read signed or unsigned (#783); TIME written as text (`5 ns`) is femtoseconds | One spelling per value across writers | #781 |
| A matched vector with a different length on each side (one writer records a port mapped to an element or slice as its whole actual) counts as equal only if the shorter is one fixed slice of the longer at every time (`sliced`), else `other_shape` | Not a difference of the simulation, but checked, not skipped | #781 |
| Names only one side records are reported (`only_a`, `only_b`), never failures | The writers record different object sets (record-typed ports, strings as one value, 2-D arrays) | #781 |
| Each yavs tier is compared with the first reference that ran the design; every other reference with that one (calibration: a difference between references is theirs) | `docs/design/simulator-discrepancies.md` majority rule | #781 |
| A difference is classified in `KNOWN`: `bug` (a yavs issue), `reference` (a catalogue entry), `noise` (dump format; fixed in the comparer where possible); anything else is `unexplained` and must be investigated. An entry holds only for a first difference in its time window, so a later regression in the same design is not absorbed | Every difference gets a reason | maintainer, 2026-10-26 |
| Three yavs tiers: `auto` compiles hot code on a thread that starts 5 ms into the run, so a short design runs interpreted under it; `llvm` compiles everything before time 0. Each yavs run sets `YAVS_COUNTERS=1` and `--verbose`, and the result shows its compiled code: functions `auto` compiled (its notes), IR instructions `llvm` compiled inline (`llvm_inline_insts`) | Equal waveforms of a tier prove something only if its code ran | #781 |
| The recorded commit is the checkout's `HEAD`: run with a binary built from that commit, after the commit is on `main` | A result must name a commit that exists | #781 |
| Reference commands come from the untracked `tools/sims.local.conf`: `waveq` (`analyse|||elab+run` of one file, `{std} {std2} {work} {file} {top} {gens} {stop_flag} {wave}`) for h2h designs, `waveq_flags` (run options writing `{wave}`, added to realbench's `real_run` `{rest}`) for realbench tests; a label without them is "not configured" | Tracked files know labels only (root rule 11) | `tools/AGENTS.md` 2a |
| Every run is `timeout 300 systemd-run --user --scope -p MemoryMax=` (6G yavs, 2G references), at most two designs at once; worlds and work directories are removed after a design, waveforms unless `--keep`; yavs runs with its own `YAVS_STD_CACHE` | Shared host (root rules 15, 20; tools 3) | #781 |

## 4. Invariants and pitfalls

- The comparer's stdout carries a line the reader library prints for some FST vector types; read the `--json` file, not stdout.
- The GHW reader of the library panics on multi-dimensional arrays: a design with a 2-D array signal cannot use a GHW reference.
- The stop time of a realbench test bounds both runs; change it in `REAL` for a shorter window (sizes in `runs/`).
- Realbench worlds are built in the realbench cache under `worlds/waveq-<pid>-...`; a killed run can leave them.

## 5. How to work here

```sh
cargo build --release -p yavs --features llvm        # the binary under test (auto tier needs llvm)
cp ~/.cache/yavs-agents/sims.local.conf tools/sims.local.conf   # untracked; needs the waveq keys
python3 -I tools/waveq/run.py --self-test
cargo test -p yavs-wave --example waveq
python3 -I tools/waveq/run.py target/release/yavs                  # everything: about 10 minutes
python3 -I tools/waveq/run.py target/release/yavs --designs neorv32_single_min --keep
cargo run --release -p yavs-wave --example waveq -- a.fst b.wave  # two files by hand
```

## 6. Coordination

- #781 (this tool), #780 (the conformance report that shows `results.json`). Findings: #782 (literals accepted without
  visible character literals), #783 (integer width in yavs FST), #784 (`'instance_name` signature; catalogue
  `subprogram-signature-instance-name`, the UVVM AXI-Stream difference, fixed: yavs spells it as sim1).
- sim2 has waveform keys for the h2h designs only: realbench builds its libraries per tool from the analysis-corpus
  manifests, and sim2 has no `corpus_cmd`, `real_elab`, `real_run` here; adding them gives a second reference on the
  real designs.
