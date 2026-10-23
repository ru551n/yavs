# Instance specialisation: design study and implementation

Status: implemented (#651, `docs/decisions.md` "Specialised copies of processes"); sections 1 to
10 are the study and its throw-away prototype (branch `spike/spec`, not on main), section 11 says
what was built and how it differs, section 12 holds the measurements of the implementation.
Question from the user: sim1 compiles the whole design ahead of time and folds every instance's generics
into that instance's code, so slice bounds, widths and signal references are constants. yavs
keeps one function per unit (`docs/design/ir.md` section 2) and reads instance facts at run time,
so compiled code cannot see them. Is per-instance code the root of the gap, and which option
should yavs take? Tracking: #532 (compiled RTL), #520 and #443 (memoisation).

All numbers are user-mode instruction counts (`perf stat -e instructions:u`), warm compilation
cache, runs pinned to one of the cores 17 to 19 (the SMT siblings of CCD0, shared with other
agents' builds, so cycle counts were noisy and are NOT used; no wall times are claimed). Builds
were done on cores 16 to 23. The sim1 numbers are its elaboration plus run, same host.

## 1. Summary

1. Instance constants are a real but small part of the gap. Specialising the process bodies per
   instance (the prototype below) removes 3 to 13 percent of the instructions of the five
   designs measured (0 percent where the run is initialisation-bound), in the interpreter and in
   compiled code alike. Against sim1 (elaboration plus run, instructions of the whole process
   tree) the `--tier=auto` runs execute, warm, 4.4 times (poc_arith_addw), 4.8 times
   (fletcher_bufferwriter_512) and 2.1 times (hdlm_lfsr) the instructions; the copies bring that
   to 4.0, 4.5 and 2.0 (table in section 6). The rest of the gap is the kernel (44 percent of the
   instructions of `fletcher_bufferwriter_512`), interpretation and the size of the compiled code
   (#532).
2. Where widths come from generics, the effect on compiled code is larger than the instruction
   count suggests: in `poc_arith_addw` under `--tier=llvm` the work done by host callbacks for
   instructions the compiled code cannot lower inline (`Cat`, `LayoutOf`, `SigRead` of run-time
   lengths) falls from 184 G to 129 G instructions (-30 percent of that part).
3. Code growth depends on the design, not on the instance count alone. Counting the optimised IR
   of the process bodies: 1.0 to 1.3 times for CPU-like RTL (`neorv32`), below 1 for designs whose
   shared code carries the memo machinery (`fletcher`), 3 to 5 times for designs made of many
   generate iterations of tiny units (`poc_arith_addw`, `sc5_fifo_repack`, `hdlm_axi_crossbar`).
   A size budget is therefore mandatory.
4. Recommendation (section 8): do option (a) in a lazy, budgeted, content-addressed form, in the
   yavs IR (so it serves LLVM and a later Cranelift backend alike), keyed on the instance data
   with the parameters of `for` generates left unfolded, off for `--dap`, `--cov` and `--vhpi`
   variable access. Do not start with whole-design ahead-of-time compilation (d).

## 2. Today

- One `Func` per process or subprogram per design unit; generics, constants and signal tables are
  instance data (`InstData`: `data` bytes, `sigs`), frozen before the simulation starts.
- The optimiser already treats instance data as read-only memory (`opt.rs` `read_only`) and folds
  constant pool descriptors, but cannot fold instance data because it does not know the bytes.
- Memoisation (`opt/memo.rs`, #443, #520, 566a6e08, 0a4e11e3): the first activation of a process
  computes the instance-constant values and stores them in the process frame, a flag selects a
  fast copy that loads them. Descriptors a process builds from instance constants live in frame
  bytes. This removes the recomputation but not the loads, the flag test, the `Alloc` sizes and
  `Cat` piece lengths held in registers, the checks that depend on them, or the dead arms of
  `if generic then`. Compiled code sees only the loads (`Gen::cat_block`, `sig_read_block` handle
  known lengths inline, a length known only from a register calls a helper or steps).
- Measured value of the memo today: `poc_arith_addw` 417.5 G to 378.7 G instructions with it
  (-9.3 percent), `fletcher_bufferwriter_512` 127.8 G to 127.7 G (nothing). The specialisation
  below comes on top of it (poc -12.6 percent more, fletcher -7.2 percent).
- Generic packages are already specialised: each instance has its own cloned HIR and `Mangled`
  identity (rule 4, `docs/design/sem.md` 5.1). Option (a) extends that treatment from packages
  to the processes of entity instances.

- `docs/design/performance.md` R7 already recommends specialising "only on the LLVM tier, for a
  function that is hot and whose generic values are literal, never in the interpreter", to protect
  start-up. The measurements here say the interpreter gains as much as compiled code (section 6),
  and that eager specialisation costs start-up (+0.95 to +1.25 G instructions on neorv32); the
  lazy form of section 8 keeps R7's start-up concern and drops the "never in the interpreter".

## 3. Ideas from sim1 (read for design only)

- It elaborates every instance with its generic actuals substituted as constants, then lowers
  each instance's processes and folds what the constants decide (slice bounds, widths, branches).
- It keys the lowered instance by the unit and the values of its generic map and lets an instance
  with an equal key reuse the code of the first one (a module cache counts instances against
  unique ones). That is the "distinct generic set" measure of section 5. The parameter of a
  `for` generate is part of a body's constants, so iterations are separate code.
- It generates code for the whole design before the run and keeps it across runs.

What we use: the key (unit plus generic values) and the observation that sharing by equal values
bounds code growth. How ours differs: the key is the frozen instance data bytes (so constants
derived from generics and from port shapes are covered without a rule per source), parameters of
`for` generates can be left out of the key, the transformation is a yavs IR pass (no per-backend
code), and it can be lazy.

## 4. Options

| | (a) copies per distinct instance data | (b) specialise at compile time only (auto tier) | (c) shared code plus a constant-folded record | (d) whole design ahead of time, cached |
|---|---|---|---|---|
| Idea | New function id per distinct (process, instance data); the optimiser folds instance data reads; interpreter and tiers run the copy | The interpreter keeps shared code; the auto tier compiles a copy per hot (process, data) | Compiler gets the record's address and layout, loads are `invariant` | Specialise everything (a), compile everything before the run, store objects |
| Measured gain, interpreter | poc_arith_addw -12.6%, fletcher_bw_512 -7.2%, hdlm_lfsr -3.0%, neorv32_single_min -3.3% (300 us; -4.4% of the run phase), osvvm_axis_random 0% | 0 | 0 | 0 |
| Measured gain, compiled (`--tier=llvm`, run phase) | poc -12.7%, fletcher -5.3%, neorv32 -3.4% | same as the compiled column of (a) for the processes that get compiled; estimated 0 to 8% of the whole run under `--tier=auto` (auto compiles little of the RTL) | estimated 0 to 2% beyond memo (loads hoisted; no folding of `Alloc`, `Cat`, branches) | (a) plus the compiled-code effects below |
| `--tier=auto` whole run | poc -8.4%, fletcher -7.2%, hdlm_lfsr -6.3% | | | |
| Compile time | none extra in the interpreter; eager optimisation costs 0.7 M instructions per copy (neorv32: init 1.38 G to 2.3 G); lazy: only for processes that activate | LLVM per distinct copy; same per-IR-instruction cost as today | none | 815 k IR instructions of neorv32 today (#532; `--tier=llvm` cold 125 G instructions against 34 G warm for 60 us), times the growth of section 5; Cranelift would cut it, not remove it |
| Cache and memory | content-addressed optimised copies in the stage cache; peak memory +50 percent at 3290 process instances in the prototype (an upper bound: eager, one raw clone per instance, and `with_spares` clones the whole `Program`; a real version needs one entry per distinct copy) | objects keyed by LLVM IR already dedupe equal copies | none | object size grows with section 5's factor (27 MB for neorv32 today) |
| Complexity | medium: a function table that grows after the runtime exists (Runtime per-function tables, `Pre`, native flags, sampler slots are sized once), cache key, policy | medium: auto policy per copy; the compile unit and the process-to-copy map | low | high, and blocked by compiled-code size (#532) |
| `--dap`, `--vhpi`, `--cov`, waves | Debugger breakpoints map a statement to a pc of the base function: keep shared code (memo is already off for a debugger). Coverage is static items plus per-instance `cov_base`, safe but keep shared to keep dead arms countable. `--vhpi` variables and `--wave-variables` use `external_variables` private copies: same rule. | same | none | whole-design compile is incompatible with a debugger session as it is today |
| Cranelift fit | good: the folding is in the yavs IR, backend-neutral; Cranelift's mid-end does little constant propagation across our idioms, so it needs the IR already folded | good if the copies are IR-level, poor if the constants are baked into LLVM IR with guards | poor: depends on LLVM `invariant.load` | best fit with a fast backend: more compiling becomes affordable; still needs the compact-code work |

Evidence behind the rows is in sections 6 and 7. (b) and (c) are estimates, labelled as such;
only (a) was built.

### Why (c) cannot reach (a)

The memo already moves the computation out of the loop. What remains in the fast copy are
dependent computations on loaded values (checks, `Alloc` sizes, lengths, branch conditions). To
fold them the compiler must know the value, which means a compile unit per distinct value set,
which is (b). A record the compiler "can see" without a copy per value set can only be hoisted.

## 5. How many distinct generic sets

Process bodies only (subprograms and package code stay shared in every option). "copies" are the
distinct (process, instance data) pairs; "copies, generate-neutral" additionally leave the
parameters of `for` generates out of the key. The IR columns count the optimised IR instructions
of the process bodies: shared = each distinct process function once, copies = every distinct copy
(measured with the prototype, `YAVS_SPEC_STATS`).

| design | process instances | distinct process functions | copies | copies, gen-neutral | optimised IR, copies / shared (all) | (gen-neutral) |
|---|---:|---:|---:|---:|---:|---:|
| neorv32_single_min | 2006 | 850 | 1723 | 1058 | 1.34 | 1.00 |
| neorv32_dual_full | 3290 | 955 | 1965 | 1289 | 1.41 | 1.13 |
| fletcher_bufferwriter_512 | 140 | 65 | 117 | 104 | 0.83 | 0.83 |
| fletcher_bufferreader | 89 | 57 | 73 | 67 | n/m (raw IR 1.15) | |
| poc_arith_addw | 359 | 40 | 359 | 356 | 3.18 | 4.09 |
| poc_cache_cpu | 336 | 61 | 105 | 71 | 0.96 | 0.82 |
| hdlm_axi_crossbar | 240 | 53 | 240 | 219 | 2.66 | 2.66 |
| hdlm_axi_fifo | 196 | 71 | 177 | 177 | raw IR 1.73 | |
| hdlm_fifo_sync, ring_buffer, axis_bfm, lfsr | 5 to 46 | = instances | = instances | | 1.00 (raw) | |
| sc5_fifo_repack | 50 | 10 | 50 | 37 | 5.27 | 3.30 |
| vai_pe_array | 167 | 37 | 167 | 37 | 1.23 | 0.55 |
| olo_axi_master_full, olo_base_cam, olo_fix_cordic_rot | 30 to 101 | 20 to 65 | 26 to 97 | | raw IR 1.3 to 1.5 | |
| osvvm_axis_random, osvvm_axi4_random, uvvm_sb_slv, uvvm_func_cov, vunit_axi_dma | 1 to 35 | = instances | = instances | | 1.00 (raw) | |
| uvvm_axistream_vvc | 33 | 14 | 33 | 33 | raw IR 1.59 | |

(The first three columns come from the 29-design scan, the optimised-IR ratios from the second
scan of the eight larger designs. "raw IR" is the lowered code before the optimiser, the first
scan's measure; the optimised ratio is lower because the shared optimised code carries the
memo machinery, roughly twice the code of the memoised block.)

Entity level (distinct generic value sets per entity instance, from the same scan): neorv32
single_min has 134 instances (that own processes) of 67 architectures; counted per region (an
instance or a generate iteration) there are 540 distinct value sets, 90 when the parameters of
`for` generates are left out. `hdlm_axi_crossbar`: 15 instances, 43 sets. `poc_cache_cpu`: 38
instances, 40 sets, 16 generate-neutral. `sc5_fifo_repack`: 11 instances, 28 sets, 11
generate-neutral. `vai_pe_array`: 6 instances, 139 sets, 7 generate-neutral. The OSVVM and UVVM
testbenches are single-instance testbenches (1.0 to 1.2 regions per architecture); their
polymorphism is in generic packages, which are already specialised per instance.

Reading: real designs have few distinct generic sets per architecture (about 1.3 in neorv32 once
`for` generate parameters are ignored), the growth comes from generate iterations whose bodies
depend on the iteration (derived constants such as `i * W`). In `poc_arith_addw` the iteration
changes constants other than the parameter itself, so leaving the parameter out does not share.

## 6. Prototype

Branch `spike/spec`, env-gated, off by default (`YAVS_SPEC=1`: key is the whole data,
`YAVS_SPEC=2`: parameters of `for` generates are not folded and not in the key,
`YAVS_SPEC_STATS=1` prints the counts of section 5). Changes:

- `yavs-ir` `opt.rs`: the optimiser may be given the frozen instance data of one instance
  (`Opt::inst`); `const_read` answers reads of `AddrSpace::Inst` from it (only for `I64`
  registers, outside the holes of unfolded parameters), so `Load`, `DopeLen`, `Pos`, `SlicePos`,
  checks, `Alloc` sizes, `Cat` parts, signal lengths and branches fold with the existing rules.
  Callee bodies cached for inlining are made without it (they are shared).
- `yavs-ir` `program.rs`: `Program::with_spares` appends one spare copy of the process code per
  process instance (a spike shortcut: the runtime sizes its per-function tables once, so spare ids
  must exist before it is created), `Program::specialize` fills the spare's optimised copy.
- `yavs-elab` `sim.rs`: after the instance data of a process's region is complete, the process
  takes the spare of the first instance with an equal key (function plus data hash); the others
  reuse it. Processes that touch shared state (`serial`) and processes of expression actuals are
  left shared.

Not done in the prototype: the lazy and cached version, subprograms, anything under a debugger.
Output compared byte for byte with the shared code: identical for the interpreter on all five
designs (modes 1 and 2 for `poc_arith_addw` and `neorv32_single_min`, mode 2 for the others) and
for `--tier=llvm` on `poc_arith_addw` and `fletcher_bufferwriter_512` (both modes on poc,
mode 2 on fletcher; neorv32 mode 1). Not run: any copy under `--dap`, `--cov`, `--vhpi`, waves.

### Instruction counts (G, warm, whole process, includes initialisation)

| design | interp shared | interp copies (1) | interp copies gen-neutral (2) | llvm shared | llvm (2) | auto shared | auto (2) |
|---|---:|---:|---:|---:|---:|---:|---:|
| poc_arith_addw | 378.7 | 331.1 | 331.7 | 358.2 | 316.7 | 392.7 | 359.8 |
| fletcher_bufferwriter_512 | 127.7 | 118.4 | 118.5 | 140.8 | 138.3 | 128.3 | 119.1 |
| hdlm_lfsr | 67.1 | 65.1 | 65.1 | | | 37.2 | 34.9 |
| neorv32_single_min, 300 us | 94.7 | 91.4 | 91.5 | | | | |
| neorv32_single_min, 60 us | 20.0 | 20.35 | 20.17 | 34.4 | 58.1 (note) | | |
| osvvm_axis_random | 3.97 | 3.98 | 3.98 | | | | |

`--tier=auto`, warm and cold (fresh world copy without compilation cache, which includes the
compile thread), against sim1's elaboration plus run (rule 23: sim1 ratios are given for the auto
tier only; the interpreter is judged by its own before/after):

| design | sim1 | auto warm shared | warm copies (2) | ratio to sim1 | auto cold shared | cold copies (2) | ratio to sim1 |
|---|---:|---:|---:|---:|---:|---:|---:|
| poc_arith_addw | 89.6 | 392.7 | 359.8 | 4.38 to 4.02 | 417.1 | 370.1 | 4.66 to 4.13 |
| fletcher_bufferwriter_512 | 26.5 | 128.3 | 119.1 | 4.84 to 4.49 | 128.4 | 119.2 | 4.85 to 4.50 |
| hdlm_lfsr | 17.4 | 37.2 | 34.9 | 2.14 to 2.01 | 40.1 | 37.6 | 2.30 to 2.16 |
| neorv32_single_min, 300 us | 38.4 | | | | 97.6 | 93.5 | 2.54 to 2.43 |

(Instructions in G, `perf stat` of the whole process tree. Two cold runs each, they agree to
0.1 percent, except neorv32 with one.) The copies also lower the cold compile part: poc cold
minus warm is 24 G shared against 10 G with copies.

IPC (same core, back to back, 300 us neorv32, interpreter, noisy host, two pairs): shared 1.45 and
1.42, copies (2) 1.45 and 1.45, so the cycles follow the instructions (-3.5 percent); the 60 us
runs (start-up phase) have IPC 2.4 for both. Auto cold cycles: neorv32 65.7 G against 65.2 G,
poc 130.9 G against 110.7 G in the first pair but 172 G against 210 G in the second (the host was
shared; instruction counts are the usable figure).

Initialisation is part of the totals. Run-phase figures (llvm, warm minus a `--stop-time=0fs`
run): poc 354.8 to 309.9 (-12.7%), fletcher 135.2 to 128.0 (-5.3%), neorv32 (60 us) 18.1 to 17.5
(-3.4%). Initialisation of the prototype is higher because every copy is optimised eagerly:
neorv32_single_min interp 1.38 G to 2.33 G (gen-neutral) or 2.63 G (all); a lazy version pays only
for processes that activate. The llvm initialisation behind the 58.1 (note) is a spike artefact: the
spike leaves one unused raw clone per process instance and `--tier=llvm` compiles every function,
so object size, link time and cold llvm totals are not representative (they are not used for
conclusions below); code size is judged by the IR counts of section 5 and compile cost by the
`--tier=auto` cold runs above.

The interpreter figures are the clean ones. Where the speed-up is: the shared process executes the
memo flag test, the slot loads, run-time `Alloc` sizes and lengths, the checks on them, and
callbacks for ops whose lengths are unknown; the copy has constants (`poc_arith_addw`'s
`SigVal{sig: 11, len: 5}` and `Alloc` of 40 bytes appear as constants, the frame shrinks from 312
bytes to 0).

### Where the instructions are (why the gain is bounded)

- `fletcher_bufferwriter_512`, interpreter: 48 percent in the interpreter loop, 44 percent in the
  kernel (13 percent in `Signal::update_effective`); specialisation takes 12 percent off the
  interpreter part and nothing off the kernel part.
- `fletcher_bufferwriter_512`, `--tier=llvm` (warm, perf samples, 141 G): compiled code 36 G, host
  callbacks 35 G, kernel 53 G, other 14 G. Copies: 33, 32, 53, 16 (total 138).
- `poc_arith_addw`, `--tier=llvm`: compiled code 101 G, host callbacks 184 G, kernel 45 G. Copies:
  106, 129, 49. The 55 G less host work is the generic-width effect; compiled code itself grows by
  5 G because the folded constants make the code do the work inline that the callbacks did.

## 7. Cost model beyond the prototype

- Compile time (rule 23, total wall including compile): copies are keyed by content, so a second
  run and equal instances cost nothing; the first run pays LLVM per distinct copy in proportion to
  its IR, about the growth of section 5. Measured with `--tier=auto` and an empty cache (above): the copies
  are cheaper in total than the shared code on all four designs (-4 to -11 percent), and on
  `poc_arith_addw` the compile part itself falls from 24 G to 10 G instructions. (`--tier=llvm`
  cold numbers are not usable, see section 6.) With `--tier=auto` only hot copies compile, so the policy (`RTL_INSTANCES`,
  `RTL_SHARE`, payback) must count by the base function: sixteen instances with different data
  must not look like sixteen cold functions.
- Optimiser time: 0.7 M host instructions per copy (neorv32: +0.95 to +1.25 G over 1058 to 1723
  copies, eager). Lazy copies are made at the first activation; a content-addressed stage-cache
  entry (kind next to `ir`, key: lowered program key, base function, masked data hash) removes it
  from warm runs.
- Memory: a copy's optimised code is smaller than the shared memo form for most processes (section
  5), but each copy has a `Pre` (predecode), live-register tables and, compiled, an object. The
  prototype's 115 MB to 177 MB for `neorv32_dual_full` at time 0 comes from eager optimisation of
  1958 copies and an unused clone per instance; a lazy version is bounded by the activated
  copies.
- Debugger, coverage, VHPI, waves: see the table in section 4; specialisation must be off when
  `keep_variables` or `external_variables` is on and, by default, with coverage, the same switch
  that turns the memo off.

## 8. Recommendation

1. Implement (a) as a pass in the yavs IR, lazily, per distinct (process, instance data), with the
   parameters of `for` generates left unfolded by default (`YAVS_SPEC=2` semantics: fewer copies,
   same gain on the designs measured), and a growth budget: specialise a process only when its
   shared optimised body has a memo plan or instance-data loads (cheap pre-scan; in neorv32 about
   two thirds of the copies come out no smaller than the shared code and would be wasted), and
   stop when the specialised IR of the design exceeds, say, 1.5 times the shared IR (poc, sc5 and
   the crossbar would then keep the largest ones shared or only specialise the hot ones in
   `--tier=auto`).
2. Make the function table append-only (needs the runtime's per-function tables to grow) instead
   of the spike's pre-sized spares; this touches `yavs-ir`, `yavs-interp` and `yavs-llvm` owners.
3. Store copies content-addressed in the stage cache; compile copies through the existing object
   cache (already keyed by LLVM IR, so equal copies collapse).
4. Do not do (d) now. The compiled code of today is not the bottleneck of the gap: forced
   `--tier=llvm` executes about the same instructions as the interpreter (1.0 to 1.1 times) and
   worse cycles, and (d) multiplies the compile cost and the object size. Revisit after #532 and
   the backend decision, when a Cranelift backend makes whole-design compiling cheap.
5. Expected result for the user's goal: 3 to 13 percent fewer instructions on RTL with generic
   widths (cold and warm, `--tier=auto` included), up to 30 percent fewer host callbacks of
   compiled code, no change for single-instance library testbenches. Against sim1 the `--tier=auto`
   ratio improves by 6 to 8 percent (poc 4.4 to 4.0 warm, fletcher 4.8 to 4.5, neorv32 2.5 to
   2.4 cold); it does not close the gap, whose larger shares are the kernel and interpretation.
   Caution on IPC: the instruction savings carried over to cycles in the one clean pair measured
   (neorv32, interpreter, same IPC); compiled code with more distinct bodies could lose IPC to
   code footprint (#532), which the compiled runs here did not isolate.

## 9. Questions for the user

1. Is a 3 to 13 percent instruction gain worth a function table that grows at run time and a new
   cache kind, or should the effort go first to the kernel share (44 percent of the instructions
   of `fletcher_bufferwriter_512`) and compact compiled code (#532)? Specialisation is a
   prerequisite for compiled code that sees widths, not a replacement for either.
2. Default for the parameters of `for` generates: unfolded (fewer copies, same measured gain) or
   folded (what sim1 does, up to 1.34 times the IR on neorv32, 3 to 5 times on generate-heavy
   small units)?
3. Budget: a cap on specialised IR as a multiple of shared IR (proposed 1.5), and whether
   `--tier=auto` may specialise a hot process on its own when the cap blocks the rest.
4. Debugger, coverage and VHPI variable access keep shared code (proposed): acceptable?
5. May the stage cache get a new entry kind for optimised copies (content-addressed)?
6. Cranelift: should the specialisation wait for the backend decision, or go first because it is
   IR-level and backend-neutral (proposed)?

## 10. Reproducing

```
git checkout spike/spec
YAVS_SPEC=2 yavs run ...            # copies, gen-neutral
YAVS_SPEC=1 yavs run ...            # copies, every instance datum folded
YAVS_SPEC_STATS=1 yavs run --stop-time=0fs ...   # counts of section 5 on stderr
```
Instruction counts: `perf stat -e instructions:u` of a warm run (two runs, the second counted;
the first run of a closure leaves only a note in the compilation cache). Worlds built with
`tools/realbench/run.py --prepare`.

## 11. As built (#651, user decision 2026-10-23)

The study's recommendation (a) is on main with the defaults of section 8 and the answers to
section 9 as proposed there. What exists, and where it differs from the prototype:

- **Copies of process bodies, content addressed** (`yavs-ir` `copies.rs`): `Program::copy_of(process,
  instance data, holes)`. The key is the process and the instance data bytes with the parameters of
  `for` generates (`holes`) zeroed, compared byte for byte (a 64-bit hash only finds candidates).
  Equal data share one copy; the iterations of a generate share a copy unless something else
  derived from the parameter (a constant `i * W`) differs. The optimiser folds the reads of the
  data into integer registers (`Opt::inst`, `const_read`) and everything the existing rules fold
  from a constant follows: lengths, slice positions, `Alloc` sizes, checks, dead arms of
  `if generic then`. A pointer or handle register keeps its load.
- **Lazy: a process switches at a suspension once it has run `YAVS_SPEC_AFTER` activations**
  (1024, section 12). The prototype made every copy at elaboration (+1 G instructions in the
  initialisation of `neorv32_single_min`); every process runs once in initialisation (LRM 14.7.5),
  so "optimise on first use" would have been as eager. `Machine::specialise` runs between two
  activations of a process that waits at its one wait statement in its root frame (the resume point
  maps to the copy's one `Suspend`; a process with several wait statements, a nested subprogram,
  shared state, coverage or no instance data stays on the shared code). The variables stay in the
  slots the lowered code gave them; the memo flag and values and the layout slots the shared
  optimised code kept behind them are zeroed, the registers are filled for the copy and a resident
  process stays resident. One compare per activation (`execs == spec_at`) is the whole cost for a
  process that never switches.
- **No function table grows; the slots are reserved.** The study asked for append-only tables.
  The first version did that (a chunked `Tail` for the program, for the runtime's `pre`, `live`
  and native flags): the lookup of a copy cost 50 to 100 instructions per activation of a switched
  process (a call out of line for `Program::func`, another for `Runtime::pre`; `neorv32_single_min`
  came out 0.5 percent worse than without copies). Now the elaborator reserves empty function slots
  (`Func::spare`) at the end of the program before the runtime is made, one per instance of a
  process that can have a copy (at most 4096), and a copy takes the next free one: a copy is a
  function like any other, the runtime's and the tier's tables have a place for it, and nothing on
  the hot paths changed (`Program::func` is byte for byte what it was; the instruction count of a
  design with copies off is within 0.1 percent of before). A design that runs out of slots keeps
  the rest shared (`spec_full`). Slots are never stored with a program.
- **Growth cap 1.5 times** the optimised IR of the shared process functions that ran
  (`YAVS_SPEC_CAP` percent, 0: none); a copy that would go over is refused and its process stays
  shared (`spec_capped`). It binds on generate-heavy designs of small units (section 12).
- **Shared code is kept** under `--dap` and anything that keeps variables (`keep_variables`), with
  `--vhpi` and `--wave-variables` (`external_variables`), with coverage counters (functions with
  `HAS_COVER`), under `--tier=llvm` (the whole program is compiled before a copy exists),
  with `YAVS_NO_IROPT`, and for serial processes (shared variables, protected objects).
- **Tiers.** Under `--tier=auto` a copy is a function with its own id: the sampler sees it, the
  policy compiles it when it is hot (the instance count of the rule for clocked processes is the
  one of its process, `Program::base_of`), and a process whose shared code is compiled already
  does not switch. Copies whose shared code is compiled at once (`--tier-heat=1,0,0`, tests) are
  compiled at their first entry.
- **Stage cache, kind `spec`**: one entry per lowering (the key of the `ir` entry, which holds the
  program, and with it the optimiser switches, the tier configuration and the build) with the
  serialised optimised copies of all runs of that design, read when the first copy of a run is
  optimised and written by `Elaborated::learn` when a run optimised a copy the entry did not have.
  Invalidated like the other stages (any change of sources, switches or build changes the key).
- **Counters** (`YAVS_COUNTERS=1`): `spec_planned` (processes that may switch), `spec_switched`,
  `spec_copies`, `spec_fresh` and `spec_loaded` (optimised in this run, read from the cache),
  `spec_capped`, `spec_full`, `spec_shared_ir` and `spec_copy_ir` (the growth, in optimised IR
  instructions).
- **Test aids**: `YAVS_NO_SPEC=1` (no copies), `YAVS_SPEC_AFTER=n` (switch at activation `n`, at
  least 2, and whatever the tier has compiled), `YAVS_SPEC_CAP=p`. `crates/yavs/tests/spec.rs`
  runs every design of `crates/yavs/tests/perf/` with copies off and with every process
  switching at its second activation, on one and four threads, and requires the same output
  byte for byte (the shadow check), plus the stage cache and the auto tier.

## 12. Measurements of the implementation (2026-10-23)

Bench builds with the `llvm` feature (`tools/build-bench.sh`) of main at 430c1c3d ("before") and of the commits of #651 ("after"),
core 9 (CCD1), `perf stat` user instructions and cycles of the whole process tree (the compile thread of `auto` included), G =
billions. The worlds are warm (two untimed runs, then the counted run; median of 3 interleaved pairs for the four designs of the
study, one pair for the rest); "cold" is a copy of the world without any compilation cache, one run. Instructions are exact, cycles
are what the loaded host gave (other agents ran; compare ratios, not values). `--tier=interp` unless the column says auto.
Default settings: switch at activation 1024, cap 1.5. Rows with 0 copies are designs whose processes read no instance data, run
once or are single-instance testbenches.

| design | interp instr | | interp cycles | | auto warm instr | | auto cold instr | | init instr (interp) | copies (switched) / IR growth | RSS MB |
|---|---|---:|---|---:|---|---:|---|---:|---|---|---|
| `hdlm_lfsr` | 66.54 / 64.48 | -3.1 % | 18.77 / 17.46 | -7.0 % | 36.92 / 34.57 | -6.3 % | 40.14 / 37.36 | -6.9 % | 0.336 / 0.336 | 1 (1) / 0.23x | 36 / 35 |
| `fletcher_bufferwriter_512` | 112.29 / 105.35 | -6.2 % | 45.67 / 41.84 | -8.4 % | 112.76 / 105.79 | -6.2 % | 112.97 / 105.95 | -6.2 % | 0.493 / 0.534 (note 1) | 72 (94) / 0.64x | 39 / 41 |
| `poc_arith_addw` | 373.94 / 333.23 | -10.9 % | 99.06 / 88.92 | -10.2 % | 294.12 / 283.12 | -3.7 % | 330.01 / 296.57 | -10.1 % | 0.172 / 0.173 | 147 (147) / 1.49x (cap) | 32 / 34 |
| `neorv32_single_min` (300 us) | 65.52 / 64.57 | -1.4 % | 25.41 / 24.94 | -1.8 % | 66.08 / 65.12 | -1.5 % | 67.92 / 67.29 | -0.9 % | 1.426 / 1.421 | 91 (171) / 0.24x | 75 / 79 |
| `sc5_fifo_repack` | 57.99 / 49.12 | -15.3 % | 16.70 / 14.12 | -15.4 % | 58.09 / 49.14 | -15.4 % | 59.57 / 50.72 | -14.9 % | 0.039 / 0.039 | 21 (34) / 1.49x (cap) | 19 / 20 |
| `poc_cache_cpu` | 25.50 / 24.27 | -4.8 % | 8.06 / 7.65 | -5.1 % | 25.78 / 24.54 | -4.8 % | 26.57 / 25.38 | -4.5 % | 0.148 / 0.149 | 49 (274) / 0.56x | 35 / 35 |
| `osvvm_axis_random` | 3.94 / 3.95 | +0.1 % | 1.27 / 1.25 | | 4.00 / 4.00 | 0.0 % | 5.78 / 5.78 | 0.0 % | 0.341 / 0.340 | 0 | 39 / 40 |
| `uvvm_sb_slv`, `hdlm_axi_crossbar`, `hdlm_axi_fifo`, `hdlm_fifo_sync`, `olo_fix_cordic_rot` | within +0.3 % | | within 2 % | | within +0.2 % | | within +0.5 % | | equal | 0 | equal |

Note 1: the analysed-unit cache of the worlds was written by the "before" binary (a cache entry is keyed by the build), so the
"after" binary re-analyses the units of this testbench at start-up (+40 M instructions in every mode, also with copies off); it is
not the copies. `tools/realbench/run.py --compare` makes a world per binary (section 12.2).

- **Copies off** (`YAVS_NO_SPEC=1`): 66.540, 112.855, 374.081 and 65.574 G for the four designs of the study, 0.00 to 0.5 percent
  above before (fletcher is the 40 M of note 1): the plumbing (one compare per activation, the copy slots, the counters) is below the noise of the
  interpreter loop's code layout, which moved the same designs by up to 1.3 percent when `Program::func` was changed in the first versions.
- **Initialisation** (`--stop-time=0fs`): unchanged within 0.4 percent. The reserved slots cost 12 M instructions of the 1.43 G of
  `neorv32_single_min` (0.8 percent) before slots were reserved only for processes that can have a copy, 5 M (0.35 percent) now; the
  prototype's eager copies cost 950 M to 1.25 G.
- **The cap** binds on two designs. `poc_arith_addw`: 312 candidate copies of 40 functions would be 3.5 times the shared IR, 147 fit; gain
  -10.9 percent with the cap, -11.3 percent without (`YAVS_SPEC_CAP=0`: 331.5 G). `sc5_fifo_repack`: 30 candidates, 2.46 times, 21 fit; -15.3
  percent with the cap, -19.6 percent without (46.6 G). No other measured design gets near it (0.64 times for `fletcher_bufferwriter_512`).
- **The threshold** (`YAVS_SPEC_AFTER`, instructions of the whole run in G for 2, 64, 256, 1024, 4096, 16384 activations):
  `fletcher_bufferwriter_512` 105.18, 105.27, 105.26, 105.34, 105.61, 106.63 (before: 112.29); `neorv32_single_min` 64.61, 64.53, 64.53, 64.57,
  64.74, 65.21 (before: 65.52); `poc_arith_addw` 343.0, 333.0, 333.1, 333.2, 333.7, 335.6 (before: 373.9). A copy costs the optimiser about
  0.7 M instructions and pays that back after a few thousand activations of the process, so the number of copies falls and the gain with
  it only above 4096; 1024 keeps every process of an initialisation (which runs each once) on the shared code with a margin and loses less
  than 0.1 percent of the gain against 64. At 2 the initialisation and the cap are spent on processes that run twice (`poc_arith_addw` 343 G).
- **Against the prototype** (section 6; its base was older and slower: neorv32 94.7 G where main is now 65.5): `poc_arith_addw` -12.6 percent
  there, -10.9 here (cap 1.5; -11.3 without); `fletcher_bufferwriter_512` -7.2 / -6.2; `hdlm_lfsr` -3.0 / -3.1; `neorv32_single_min` -3.3 /
  -1.4 (the memoisation and the other interpreter rounds since took most of what the copies gained there: its hot processes read little
  instance data; of 847 process functions 287 take an instance-data address and 171 process instances reach 1024 activations in 300 us);
  OSVVM 0 / 0.
- **RSS** within 4 MB everywhere (the copy slots and one `Pre` per copy; `neorv32_single_min` +4 MB, 5 percent).
- **The stage cache** (kind `spec`): 44 KB for `sc5_fifo_repack` (21 copies), 71 KB for `poc_cache_cpu` (49), 220 KB for
  `poc_arith_addw` (147) and 334 KB for `neorv32_single_min` (91), next to `ir` entries of 0.09 to 5.2 MB; designs whose lowering is not
  cached (`fletcher_bufferwriter_512`, `hdlm_lfsr`: no `ir` entry) store none and optimise their copies in every run (0.7 M instructions
  each, 50 M of 105 G for fletcher). A warm run reads the copies of the earlier runs instead of optimising them (`spec_loaded` =
  `spec_copies`, `spec_fresh` = 0; `tests/spec.rs`).
