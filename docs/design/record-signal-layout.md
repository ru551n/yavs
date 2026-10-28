# Record signal layout: packs of leaves in one kernel signal

Status: design, 2026-10-24, #686; option A chosen (user, 2026-10-24); steps 1, 2 and 4 implemented (step 4 without `Ctx::codes` and the whole-record wait). Phase 1 of the "struct layout for
record signals" task: what changes, what stays, and the steps to land it.

## 1. Problem

A record signal is one kernel signal per leaf (a scalar field, or an array of scalars; ir.md 4,
"Leaves"). Every whole read and every whole drive walks the leaves: per leaf a signal reference,
a read or a drive, a field address, a store or a load, and behind the drive the kernel's
`assign_code_fast`, `commit_code`, `commit_out` and `finish_direct`.

Measured (bench build, `--tier=interp --threads 1`, `perf record -e instructions:u`) on a synthetic
copy of the register record of the LEON3 testbench's integer unit (`grlib_leon3mp`): 233 leaves in seven sub-records (two
thirds `std_ulogic`, the rest `std_logic_vector` of 2 to 32 bits, a few naturals and one
enumeration), `comb (r)` doing `v := r`, 42 field updates and `rin <= v`, a clocked `r <= rin`,
20 000 cycles: 9.81 G instructions, about 490 000 per clock cycle, about 525 per leaf operation
(four leaf passes per cycle: fill of `v`, `rin <= v`, read of `rin`, `r <= rin`).

| Part | Share | Symbols |
|---|---|---|
| IR execution | 55% | `run_ops` 40, `put` 3, `sig_read_scalar` 3, `op_lcopy_drive` 3, `memmove` 2, `drive` 1 |
| Kernel | 37% | `finish_direct` 7, `assign_planes_fast` 6, `commit_code` 5, `read_into` 5, `commit_out` 5, `assign_code_fast` 4, `cycle` 2, `assign_bytes` 2, `wake_sole` 1 |

The IR part is what the gather and scatter instructions of #677 remove (one instruction per whole
read or drive, a Rust loop over a table). The equal-value drive dropping of the kernel lane
(#676/#519 agent) removes the commit and finish of leaves whose value did not change (190 of the
233 here), not the call that finds out. What neither removes is the per-leaf kernel work itself:
one call, one driver lookup, one signal lookup and one entry per leaf, and per changed leaf one
commit, one finish and one changed run. That is what this layout removes: a pack of up to 64
leaves is one plane block, so it is read, driven, committed and finished with word operations.

## 2. What sim1 does

The task assumed that sim1 stores a record signal contiguously. It does not (study of its
runtime and lowering for this design, ideas only, rule 22): a record signal is a tree of separate
runtime signals, one per scalar or homogeneous-array leaf, each with one element size (a
`std_logic` element is one byte) and its value, last value and driving value in one buffer. A
whole-record read is one copy per leaf by a generated helper, a whole drive one waveform call per
leaf. Its speed comes from elsewhere:

- compiled code reads a leaf's current value straight from memory (no runtime call), and tests
  `'event` of fully tested signals from a cached flag;
- a leaf is split lazily into "nexuses" (runs of elements with the same sources and waiters) at
  the first sub-range access; whole-signal accesses take the first nexus with no search;
- a "fast driver" (one source, nothing pending) copies the value into its one waveform slot and
  drops the update when the value is unchanged and nobody observes activity;
- `std_logic` resolution is a memoised table.

yavs already has the equivalents of all four (direct reads of codes from compiled code, segments,
direct and quick transactions, plane-wise `std_logic` resolution). So matching sim1 is not the
argument; the argument is the per-leaf kernel work of section 1, which sim1 pays too and which a
pack avoids. The gains below are our own measurements and estimates, not a comparison.

## 3. The layout

### 3.1 Packs and the leaf map

- The elaborator's logical model does not change: `LeafId` stays one per field leaf, with its
  object, field path, type, initial value; `NetGraph::select` (rule 2, the one sub-element map)
  still returns `(LeafId, Range)` with logic leaves mirrored inside the leaf.
- New: a physical map `NetGraph::loc: Vec<LeafLoc>` per leaf, `LeafLoc { signal: SignalId, base:
  u32 }`, and its reverse per kernel signal (first leaf, number of leaves; the bases ascend). Every
  place that turns a leaf into a kernel reference goes through one helper, `NetGraph::kref(leaf,
  range) -> SigRef` (`signal.slice(base + range.offset, range.len)`), and every place that turns a
  kernel `(signal, range)` back into names through `NetGraph::leaves_at(signal, range)`. Kernel
  `SignalId` order is pack order, which is first-leaf order: deterministic, one per design and
  configuration.
- A **pack** is a run of consecutive leaves of one signal object (after port collapse) stored in
  one kernel signal, leaf `k` at elements `base_k .. base_k + len_k`, the bases the running sum of
  the lengths. The instance signal table already carries a base (`InstSig::off`, today the start
  of a port collapsed into part of its actual) and so does the compiled tier's `TierSig::off`:
  `off = loc.base + part offset`. The IR, the lowering and every `SigVal` stay as they are.

### 3.2 Which leaves form a pack

A kernel signal has signal-wide properties that decide its fast paths: `independent` (no port
edge, no composite group, not a serial resolution) and with it `DriverInfo::direct`/`bdirect`;
`inactive` (no implicit signal has it as a prefix); `skip_ok` (no guard, user resolution, group,
implicit signal or converting port in its port component); `lean`; the codes rule; the element
stamps compiled code reads (64 elements or fewer). One odd leaf would switch them off for the
whole pack. So leaves join a pack only when all of these hold for every leaf:

| Rule | Why |
|---|---|
| element kind logic (`std_ulogic`, BIT, BOOLEAN) | bit-planes everywhere (#83, user 2026-10-18); byte leaves come later (step 6) |
| resolution none or `std_logic` (`resolved`) | one resolution per kernel signal; an unresolved leaf has at most one source per element (elaboration checks LRM 14.7.2 before the build), and `std_logic` resolution of one source is the identity, so both behave the same in a `std_logic` pack. A leaf with a user resolution function (the verification libraries' resolved transaction records) or in a composite resolved as a whole stays alone |
| no guard, not implicit, not the prefix of an implicit signal, not a force/release target | signal-wide in the kernel (`guard`, `inactive`, the dirty-range path of a forced cycle) |
| the same port situation: no port edge at all, or (step 3) the same edges' kind and peer objects | `independent`, `skip_ok` and port levels are per signal; a mixed pack would lose the direct path for its port-free leaves |
| total at most 64 elements; a leaf of more than 64 elements stays alone | one plane block: codes always kept (`sync_codes`), element stamps offered to compiled code, `vquick`/`OutV` of one block, no `blk_active`/`blk_event`/lean machinery; a 233-leaf record of mostly scalars is four or five packs |

A record of identical type on both sides of a port gets identical packs (the rules depend on the
type, the constraints and the per-leaf facts), so its per-leaf port edges coalesce (step 3).

### 3.3 Kernel state per pack

Nothing new in the kernel for steps 1 to 3: a pack is an ordinary logic signal of up to 64
elements.

- **Value planes.** Effective `cur`, `driving`, `last_value`, per-driver values, force and port
  layers are the signal's `Buf`s; a pack's leaves are element ranges of them. Codes
  (`Signal::codes`) are one byte per element, exactly the IR form of a scalar `std_ulogic` field.
- **Drivers.** `DriverDecl` stays per leaf, and through step 3 so does the kernel driver (a drive
  without a target names the leaf). From step 4, the declarations of one process whose
  leaves are adjacent in one pack become one kernel driver (`Builder::driver` of the run); each IR
  driver slot keeps its leaf range and names the merged `DriverId`, and a drive targets its leaf
  range inside it, as `s(i) <= x` already targets part of a driver. One convention for
  `ProcDriver::range`: pack-relative, the base added once, when the slot is made (today the
  collapse remap and `driver_ref` both add an offset; correct on the shapes tried, see 7). A process
  that drives `r.a` and `r.c` but not `r.b` gets two drivers. Merging two drivers of one process
  on disjoint elements changes no source set: per element it is the same one source.
- **Resolution** is per segment as today; a pack whose leaves have one source each is all direct
  drivers and never resolves.
- **Events, `'event`, `'last_event`, `'last_value`.** Per element stamps (a pack has at most 64
  elements, so never lean and never block records). `'event` of a whole record is "any element of
  its ranges", `'last_event` the most recent, `'last_value` a range read of the `last_value`
  planes: a few range queries instead of one per leaf.
- **Sensitivity on one leaf** is a waiter on that leaf's range of the pack; signals of 9 to 1024
  elements keep a waiter list per element, so an event on `r.a` looks at `r.a`'s waiters only. A
  process sensitive to the whole record registers one range per leaf as today (resident, so
  nothing per activation); step 4 makes it one range per pack.
- **Ports.** Collapse runs first and is unchanged: a collapsed formal's leaves are its actual's
  leaves, so it follows their `LeafLoc`. An uncollapsed whole-record port is one kernel edge per
  pack pair once consecutive leaf edges with consecutive ranges on both sides are coalesced; a
  partial association (`p.a => x, p.b => y`) keeps one edge per leaf, and the port rule of 3.2
  keeps leaves with different edges out of one pack.
- **Partial drives and reads** (a leaf, a slice of a leaf, an element of a leaf, a sub-record) are
  ranges of the pack: `SigVal` plus the base. A sub-record field of a record is a contiguous range
  of the parent's packs (leaf order is depth first, and packing keeps order), so a whole
  sub-record read is a few ranges too. An element of an array of records stays strided (one part
  per leaf, struct of arrays) as today.
- **`--threads`.** Same semantics: processes write their own drivers; the commit is serial; the
  parallel update of independent signals has fewer, larger tasks. Output stays byte-identical at
  any thread count because the pack choice is a function of the elaborated design only.

### 3.4 Both tiers

- Interpreter: `InstSig::off` carries the base; reads (`elem_code`, `read_into`), edges, waits and
  drives are unchanged. `InstSig::ty` must carry the leaf's length, not the pack's.
- Compiled tier: `TierSig` carries `off` and the pack's codes, planes and stamps (offered because a
  pack has at most 64 elements), so compiled single-field reads, `'event` and edges stay inline.

### 3.5 Waves, VHPI, DAP, coverage, diagnostics

All name objects and fields by the logical model, so only the lookups change, through
`kref`/`leaves_at`:

- waves: `Src::Sig` keyed by leaf; `value_changes(signal, ranges)` goes through the reverse map to
  the leaves' variables (the recorder is already range based);
- VHPI handles and DAP variables keep `ObjId`/`LeafId`; reads, forces and deposits use `kref`
  (they ignore `SigObj::part` today; the same helper fixes that if it is a bug, see 7); DAP signal
  watches compare the leaf's range, not the whole kernel signal;
- coverage toggles: per leaf base plus the bit, re-keyed to the kernel signal;
- kernel names: a pack's kernel name is the path of the innermost record holding its leaves; the
  elaborator names its fields as parts (`SimControl::name_parts`), and the hang, stuck and delta
  loop reports work per part (waits, drivers, values and events of the field), so they read as
  before packs (#704). Kernel errors that print a signal's name do not reach a pack (they are
  about user resolution functions, guards and ports with several sources, which do not pack).

### 3.6 Bulk access: what gather and scatter map onto (#677)

The gather and scatter tables of #677 are built once per process (declarative part). Entries
should be **runs**, not leaves: `(kernel signal, element range, IR offset, field shape)` where
the shape says "scalar logic codes", "logic vector planes", "bytes of size n". The table builder
resolves each leaf through the instance signal table and joins leaves that are adjacent in one
kernel signal and adjacent in the IR layout. Without packs a run is one leaf; with packs a run is
up to a whole pack, so the instruction does not change, its table shrinks.

Per run, at run time:

- read: scalar `std_ulogic` leaves are bytes in `Signal::codes`, and consecutive scalar fields are
  consecutive bytes in the IR record (natural alignment of one-byte fields), so a run of them is
  one `copy_from_slice`; a vector leaf is one `LCopy` of its plane range (the IR and the kernel use
  the same plane orientation, element 0 rightmost);
- drive: the run's plane block is built from the IR fields (one `put_plane_code` per scalar, one
  funnel copy per vector) and assigned with one `Ctx::assign_planes_now`/`assign_blocks` per pack:
  one `OutV` entry, one `commit_planes_run`, one `finish_pend` word, events by XOR; an unchanged
  pack is one compare of four words (the equal-value drop at block granularity);
- wait: one `wait_on` per run.

The only kernel additions are a slice view of a signal's codes (`Ctx::codes(signal, range)`, the
codes already kept) and, for the drive, nothing (one block assignments exist).

## 4. The other option, which needs the user

Option B, a true single copy: store a record signal as one byte blob in the IR record layout
(scalars as bytes, vector fields as padded plane vectors), so a whole read is one `memcpy` with no
per-field work at all. It needs a new kernel element kind with a field table (per-element sizes,
mixed planes and bytes), loses the plane-wise resolution, event XOR and the logic fast paths for
those signals, and contradicts the decision "bit-planes everywhere" (#83, user 2026-10-18).
Option A (packs, this document) keeps every existing decision and pays per field only a byte copy
or a funnel shift. Recommendation: A; revisit B only if A's measured per-field cost is the
bottleneck after step 4.

## 5. Migration, each step landable

Every step keeps difftest 505/505 on all tiers and the verification-library suites green, adds or moves
`crates/yavs/tests/perf.rs` bounds, and lands only if `realbench --compare` instructions do not
regress at `--tier=interp` and `--tier=llvm`. A switch `YAVS_PACKS=off` (like `YAVS_LEAN_EVENTS`)
gives the old one-leaf-per-signal layout for A/B runs and as a fallback while the steps land;
coverage, a debugger and VHPI keep the packs (nothing observable depends on them).

| Step | What | Expected gain |
|---|---|---|
| 0 | Measurement (section 1) and this document | none |
| 1 | `LeafLoc` map with identity values; every consumer (build.rs call sites, `inst_data`, `actual_process`, collapse output, lean, wave, coverage, VHPI, DAP, hang and stuck names) goes through `kref`/`leaves_at`; `ProcDriver::range` convention | none (level instructions); prerequisite |
| 2 | Packs of logic leaves of record signals with no port edge (internal registers such as `r`, `rin` of the integer unit; a port collapsed into them follows them); drivers stay one per leaf, idle-skip reads armed by their range | rec233: kernel per-leaf commit and finish become per pack: estimate 10 to 15% of rec233. LEON3: iu, dctrl, ictrl register records: 2 to 4%. neorv32: internal control records, 1 to 3%. fletcher, the verification-library testbenches: about 0 (wide vectors; user-resolved transaction records; testbench-bound) |
| 3 | Packs with port edges (the same edges and peers), coalesced edges | AHB/APB record ports of a processor system, AXI record tests of two open-source libraries, a stream-processing project's records: fewer port edges and port updates per cycle, estimate 2 to 8% on the `records` category, LEON3 1 to 3% |
| 4 | Bulk access over runs: #677's gather/scatter tables of runs, `Ctx::codes`, adjacent drivers of a process in a pack merged (a drive without a target then names its leaf range), one assignment per pack, one wait per run | rec233: kernel share 37% to under 5% of what remains after #677; LEON3: `comb`'s record work (19% of the run before #677) to 1 to 2%, together with #677 about 15% of LEON3 |
| 5 | Measure, then decide: raise the 64-element cap (a 233-scalar pack would be one read of 233 codes, but loses the per-element stamps compiled code reads) | open; measured on LEON3 and neorv32 |
| 6 | Byte leaves (enumerations, integers) in packs of one element size, with a multi-element byte fast path (today only one byte element has `bquick`/`OutB`) | small: LEON3's register records have few byte fields; FSM-heavy records more |

The estimates for steps 2 and 3 split the kernel share of section 1 between this layout and the
equal-value dropping (which removes the commit and finish of unchanged leaves either way); they
count each saving once. Step 4 depends on #677 landing first; steps 1 to 3 do not.

## 6. Risks

- One odd leaf turning off a pack's fast paths: guarded by the rules of 3.2; `YAVS_COUNTERS`
  gets `packs` and `packed_leaves` so perf.rs can hold a floor.
- Fewer `wake_sole` and `pend_one` hits (several leaves of a pack changing in one cycle take the
  word path instead): expected to be cheaper, measured per step.
- Process sensitivity to single fields of a pack: per-element waiter lists keep it O(waiters of
  the element).
- Consumers that index by kernel signal and assume one leaf: inventoried for step 1 (wave
  `by_leaf`/`solo`, coverage `base`, DAP `Sig`/watches, VHPI `Origin::Signal`, kernel hang names).

## 7. Notes found on the way

- The collapse remap adds a part's offset to driver ranges and `driver_ref` adds `InstSig::off`
  again; a collapsed `out` formal on `w(5 downto 2)` driven as a whole and element by element gave
  the right values, so the target path does not reach it. Step 1 fixes the convention anyway.
- VHPI and DAP do not read `SigObj::part` (not verified to give wrong values); step 1 routes them
  through `kref`.
