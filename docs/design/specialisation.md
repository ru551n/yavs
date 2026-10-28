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
| `--dap`, `--vhpi`, `--cov`, waves | Debugger breakpoints map a statement to a pc of the base function: keep shared code (memo is already off for a debugger). Coverage is static items plus the running instance's `cov_base` (read at run time, never baked into a copy): copies, memoised blocks and inlined functions keep their counters (#699); the counters of an arm a copy folds away are dropped with it (the count stays 0). `--vhpi` variables and `--wave-variables` use `external_variables` private copies: same rule. | same | none | whole-design compile is incompatible with a debugger session as it is today |
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
generate-neutral. `vai_pe_array`: 6 instances, 139 sets, 7 generate-neutral. The verification-library
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
  `keep_variables` or `external_variables` is on. With coverage it stays on (#699): a copy is keyed by
  instance data only and counts into the `cov_base` of the instance that runs it.

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
  maps to the copy's one `Suspend`; a process with several wait statements, coverage or no instance data stays on the shared code; a nested subprogram or shared state do not keep it from a copy since #721). The variables stay in the
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
- **Growth cap 2.5 times** (user, 2026-10-24; 1.5 times at first) the optimised IR of the shared process
  functions that ran (`YAVS_SPEC_CAP` percent, 0: none); a copy that would go over is refused and its
  process stays shared (`spec_capped`). It bound on two designs at 1.5 and binds on one of the realbench
  set at 2.5 (`poc_arith_addw`: 244 of 312 candidates; section 12, "The cap raised").
- **Shared code is kept** under `--dap` and anything that keeps variables (`keep_variables`), with
  `--vhpi` and `--wave-variables` (`external_variables`), under `--tier=llvm` (the whole program is compiled before a copy exists),
  with `YAVS_NO_IROPT`, and for serial processes (shared variables, protected objects).
- **Tiers.** Under `--tier=auto` a copy is a function with its own id: the sampler sees it and the
  policy compiles it like any function, but every decision about it is the process's (#661, 2026-10-24):
  the sampled time of a process and all its copies is one number (`Worker::group`, by `Program::base_of`),
  and the threshold, the payback rule, the band wait of clocked processes, the share and instance rules
  (`cold`) and the budget factor go by that number and by the length and class of the process's shared
  code, so a process of many instances with distinct data is as hot as the same process without copies and
  is classified as it. When the process is hot every copy that was seen running is a candidate (and one
  that starts running later, as its instance switches), each compiled from its own code, so the copy that
  runs is the copy that is compiled; a trial that takes the compiled code of one copy back keeps the copies
  not compiled yet interpreted too. A process whose shared code is compiled switches to its copy too
  (`Tier::compiles_copies`, #679). A process did wait 150 ms after the tier was set before its first try
  (`SPEC_TIER_WAIT`, until #675): the tier takes about 30 ms of samples to compile a process, and a compiled
  process beats its interpreted copy by far (`hdlm_lfsr`: 31.7 G compiled shared code against 66.9 G
  interpreted copy). Compiled shared code switching to compiled copies made the wait unnecessary and
  `sc5_fifo_repack` warm 7 to 10 percent slower with it. Compiled copies are named, and so keyed in the compilation cache, by what they are for
  (`Program::copy_tag`: a hash of the process and the masked instance data, `f<process>c<tag>` in the
  module), not by their function ids, which follow the order the processes switched in and so differ between
  runs with `--threads` above 1: the same copy is the same module in every run and is linked from the cache.
  Copies whose shared code is compiled at once (`--tier-heat=1,0,0`, tests) are compiled at their first entry.
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

Bench builds with the `llvm` feature (`tools/build-bench.sh`) of main at d72ffdd9 ("before") and of the commits of #651 ("after"),
core 9 (CCD1). Instructions are user-mode instructions of the whole process tree, the compile thread of `auto` included (`perf stat`,
exact); the figures come from `tools/realbench/run.py --compare` (a world per binary, medians of 3, phase `run`), G = billions.
"warm" has an untimed run before the timed ones, "cold" is a world without any compilation cache. The host was loaded by other agents and
cycles and wall times moved by 10 to 30 percent between runs of the same binary (a pinned core shares with its SMT sibling, and under
`--tier=auto` the compile thread shares the core: the same binary gave 16.8 to 26.6 G cycles for the same instructions), so the ratios
of instructions are the result; cycles are given where three interleaved pairs agree. Settings: switch at activation 1024, cap 1.5.

| design | interp instr | | auto warm instr | | auto cold instr | | copies (switched) / IR growth | RSS MB interp |
|---|---|---:|---|---:|---|---:|---|---|
| `poc_arith_addw` | 372.93 / 331.99 | -11.0 % | 287.28 / 265.70 | -7.5 % | 329.24 / 304.46 | -7.5 % | 147 (147) / 1.49x (cap) | 33 / 35 |
| `sc5_fifo_repack` | 57.64 / 48.63 | -15.6 % | 47.27 / 49.58 | +4.9 % (note 2) | 57.07 / 50.46 | -11.6 % | 21 (34) / 1.49x (cap) | 21 / 20 |
| `fletcher_bufferwriter_512` | 106.36 / 98.64 | -7.3 % | 106.92 / 99.26 | -7.2 % | 106.88 / 99.23 | -7.2 % | 72 (94) / 0.64x | 40 / 41 |
| `poc_cache_cpu` | 25.36 / 24.07 | -5.1 % | 25.76 / 24.44 | -5.1 % | 26.21 / 24.99 | -4.7 % | 49 (274) / 0.56x | 35 / 37 |
| `hdlm_lfsr` | 69.02 / 66.77 | -3.3 % | 31.90 / 32.53 | +2.0 % (note 2) | 35.91 / 36.23 | +0.9 % | 1 (1) / 0.23x | 37 / 36 |
| `neorv32_single_min` (300 us) | 64.97 / 63.36 | -2.5 % | 65.61 / 63.98 | -2.5 % | 67.78 / 65.70 | -3.1 % | 91 (171) / 0.24x | 75 / 79 |
| `osvvm_axis_random` | 3.94 / 3.94 | 0.0 % | 4.00 / 4.00 | 0.0 % | 5.06 / 5.06 | 0.0 % | 0 | 40 / 41 |
| `hdlm_axi_crossbar`, `hdlm_fifo_sync`, `olo_fix_cordic_rot` | within +1.1 % | | within +0.3 % | | within +2 % | | 0 | equal |

Geometric mean over these ten tests: instructions 0.96 (`interp`), 0.98 (`auto`), 0.97 (`auto-cold`); CPU time 0.84 / 0.91 / 0.97 and wall
0.84 / 0.92 / 0.96, which the noise above inflates (`poc_cache_cpu` ran 2.25 s against 1.53 s with 5 percent fewer instructions).
Interleaved three-pair medians of cycles (`--tier=interp`): `hdlm_lfsr` 29.30 / 27.96 G (-4.6 %), `poc_arith_addw` 98.65 / 88.39 (-10.4 %),
`neorv32_single_min` 25.86 / 25.21 (-2.5 %); `tools/bench` h2h designs: geometric mean of CPU time 1.01, instructions equal.

Note 2: with copies the tier does not compile these two designs' clocked processes the way it does without them (`sc5_fifo_repack`: 45.1 G
with `YAVS_NO_SPEC=1`, 49.6 G with copies, against 57.6 G interpreted without copies; `hdlm_lfsr` single process: 30.5 to 32.7 G against
31.7 to 31.9 G, the copy of its one process compiles late). #661 holds the analysis; the wait of `SPEC_TIER_WAIT` (150 ms) is the first
step and made `hdlm_lfsr` better than main in the interleaved run (30.46 against 31.66 G) but not `sc5_fifo_repack`. (Settled by counting copies by their process, section 12 "The tier policy by process".)

- **Copies off** (`YAVS_NO_SPEC=1`): 68.931, 106.301, 372.809 and 64.935 G for `hdlm_lfsr`, `fletcher_bufferwriter_512`,
  `poc_arith_addw` and `neorv32_single_min` against 68.926, 106.243, 372.683 and 64.913 before (+0.0 to +0.1 percent): the plumbing (one compare
  per activation, the copy slots, the counters) is below the noise of the interpreter loop's code layout, which moved the same designs by up to
  1.3 percent in the first versions when `Program::func` itself was changed.
- **Initialisation** (`--stop-time=0fs`): equal within 0.4 percent for every design above (`neorv32_single_min` 1.418 / 1.416 G, `poc_arith_addw`
  0.172 / 0.173); the prototype's eager copies cost 950 M to 1.25 G. (An earlier run of `fletcher_bufferwriter_512` showed +40 M: the analysed-unit
  cache of the benchmark world was written by the other binary; compare binaries in worlds of their own.)
- **The cap** binds on two designs. `poc_arith_addw`: 312 candidate copies of 40 functions would be 3.5 times the shared IR, 147 fit; gain
  -10.9 percent with the cap, -11.3 percent without (`YAVS_SPEC_CAP=0`: 331.5 G). `sc5_fifo_repack`: 30 candidates, 2.46 times, 21 fit; -15.3
  percent with the cap, -19.6 percent without (46.6 G). No other measured design gets near it (0.64 times for `fletcher_bufferwriter_512`).
- **The threshold** (`YAVS_SPEC_AFTER`, instructions of the whole run in G for 2, 64, 256, 1024, 4096, 16384 activations, an older base): 
  `fletcher_bufferwriter_512` 105.18, 105.27, 105.26, 105.34, 105.61, 106.63 (without copies 112.29); `neorv32_single_min` 64.61, 64.53, 64.53,
  64.57, 64.74, 65.21 (65.52); `poc_arith_addw` 343.0, 333.0, 333.1, 333.2, 333.7, 335.6 (373.9). A copy costs the optimiser about 0.7 M
  instructions and pays that back after a few thousand activations of the process, so the number of copies falls and the gain with it only above
  4096; 1024 keeps every process of an initialisation (which runs each once) on the shared code with a margin and loses less than 0.1 percent of
  the gain against 64. At 2 the initialisation and the cap are spent on processes that run twice (`poc_arith_addw` 343 G).
- **Against the prototype** (section 6; its base was older and slower: neorv32 94.7 G where main is now 65.0): `poc_arith_addw` -12.6 percent
  there, -11.0 here (cap 1.5; -11.3 without); `fletcher_bufferwriter_512` -7.2 / -7.3; `hdlm_lfsr` -3.0 / -3.3; `neorv32_single_min` -3.3 / -2.5
  (the memoisation and the other interpreter rounds since took part of what the copies gained there: its hot processes read little instance data;
  of 847 process functions 287 take an instance-data address and 171 process instances reach 1024 activations in 300 us); verification-library testbenches 0 / 0.
- **RSS** within 4 MB everywhere in the table (slots are 0.9 KB each and sized by the cap; one `Pre` per copy; `neorv32_single_min` +4 MB, 5 percent;
  a synthetic design of 4096 one-process instances +0.7 MB of 35).
- **The stage cache** (kind `spec`): 44 KB for `sc5_fifo_repack` (21 copies), 71 KB for `poc_cache_cpu` (49), 220 KB for
  `poc_arith_addw` (147) and 334 KB for `neorv32_single_min` (91), next to `ir` entries of 0.09 to 5.2 MB; designs whose lowering is not
  cached (`fletcher_bufferwriter_512`, `hdlm_lfsr`: no `ir` entry) store none and optimise their copies in every run (0.7 M instructions
  each, 50 M of 100 G for fletcher). A warm run reads the copies of the earlier runs instead of optimising them (`spec_loaded` =
  `spec_copies`, `spec_fresh` = 0; `tests/spec.rs`).

### The cap raised from 1.5 to 2.5 (2026-10-24, user decision)

Bench builds with `llvm` (`tools/build-bench.sh`), `tools/realbench/run.py --compare` of main at cf32383b and the same with
the default cap changed, core 9, medians of 3, user instructions of the whole process tree in G (exact); RSS is the peak of the
whole tree in MB. The cap binds only on `poc_arith_addw` now (244 copies of 312 candidates, 68 refused; 8722 shared and 21766 copy IR
instructions; its interpreter run 331.2 G against 331.5 G with no cap at all). Interpreter: `sc5_fifo_repack` 48.63 / 46.12 (-5.2 %,
i.e. -20.0 % against 57.64 without copies; with no cap at all it was 46.6), `poc_arith_addw` 331.80 / 330.70 (-0.3 %, -11.3 % against
none), the other eight within +-0.3 % (`olo_fix_cordic_rot` +1.1 % is the noise of its 3.7 G). Peak RSS (interp): `sc5_fifo_repack`
21 / 23 MB, every other test within 1 MB. Initialisation (`--stop-time=0fs`, warm, instructions): equal within 0.5 % on nine tests
(`neorv32_single_min` 1410 / 1417 M, the extra slots); `fletcher_bufferwriter_512` 492 / 532 M, which is not the slots (the same with
`YAVS_NO_SPEC=1`, with `YAVS_SPEC_CAP=150`, and in every later build; deterministic per binary, not understood, 0.04 % of the run).
Under `--tier=auto` the instructions depend on what the tier compiled when, so they move with the policy and not with the cap
(`poc_arith_addw` warm 285.1 / 259.2, `sc5_fifo_repack` warm 43.2 / 47.2: #661).

### The tier policy by process (#661, 2026-10-24)

`Worker::group` (`auto.rs`) adds the sampled interpreted time of a process and of all its copies; the threshold, the payback rule,
`wants_probe`, the band wait and `cold` (share and instance rules) read it, and `is_rtl`, `base_len` and `cost` classify and price a copy as
the shared code of its process. Every copy seen running is a candidate once the process is hot. Compiled copies are named by
`Program::copy_tag`. Measured with `--tier=auto` as the user's tool runs it (a world per sample made fresh, two untimed runs, then the
timed one; samples of the binaries interleaved so that the load of the host falls on all; bench builds with `llvm`; core 10; instructions of the
whole process tree in G), because a single run says little: the state the warm-up leaves in the compilation cache is bimodal (about 35 or
about 47 G on `sc5_fifo_repack`, whatever the binary or the policy) and which one a world reaches depends on how long the compile thread
took in its first runs.

| design (8 samples) | before the change, old policy | this change | the same without copies (`YAVS_NO_SPEC=1`) |
|---|---|---|---|
| `sc5_fifo_repack` warm | median 35.6 (mean 38.8) | median 35.6 (mean 38.5) | median 38.4 (mean 38.9) |
| `hdlm_lfsr` warm | 31.95 (31.94 to 31.97, two samples 33.9) | 31.96 (two samples 33.9) | 33.1 |
| `sc5_fifo_repack` cold (6) | 52.5 | 53.4 | |
| `poc_cache_cpu` cold (6) | 27.0 | 26.7 | |

The first column is main at 01c2f0b7 (the cap change on top of the #634 commits 2f07eb62 and f313b146). On the base this change was written
on, before those commits, the old policy ran `sc5_fifo_repack` warm at 46.9 G in six of eight worlds (median 46.9, mean 46.2; the case of the issue:
copies worse than no copies, 45 G), and a world of the new policy on that base ran 35.5 G in each
of five timed runs; on the current base the old policy has caught up by itself and the new one is equal to it (two sets of eight with
the new build: median 36.1 and 35.6, its no-copies variant 44.5 and 38.4: the spread between sets is the host's load). The policy no longer
depends on that luck: the copies of a process are compiled once the process is hot, whichever of them the sampler saw first. `hdlm_lfsr`
compiles its testbench's `main`, which is not a copy, and is not changed by the policy (its +5 percent against the 30.44 G of cf32383b
comes from the #634 commits, #663). `tools/realbench/run.py --compare` (medians of 3, one run per cell on a host at load 20 to 30, so the
timing-driven cells move by 5 to 10 percent): interpreter instructions equal within 0.1 % on all ten tests (the change touches
`--tier=auto` only), peak RSS equal within 2 MB.

Unit test: `tests/objcache.rs` compiles the copies of one process for `G = 3, 4` and then `G = 4, 3` (so with swapped function ids) and
requires that the second compile stores nothing (it fails with the id in the symbol names: 8 stored against 6).

### Wider specialisation, measured and not landed (2026-10-24)

User decision 2026-10-24, item 3: measure what (a) processes with several wait statements and (b) folding of pointer-typed instance data
add, and land each that gains.

(a) A resume point maps to the copy's by statement (`Suspend` of the same `StmtId`; the process qualifies when every resume point is a
`Suspend` and no two are at the same statement). Over the 42 realbench testbenches (`spec_planned` at time 0, a process that may switch)
it admits 1 more process in `hdlm_ring_buffer` (16 to 17), 1 in `fletcher_bufferwriter_512` (118 to 119), 1 in
`fletcher_bufferwriter_prim32` and 8 in `grlib_leon3mp` (454 to 462), none in the other 38 (`neorv32` 1169 to 1935, `poc_arith_addw` 356,
`sc5_fifo_repack` 44, and so on are all single-wait): the clocked and combinational processes of RTL wait on a sensitivity list, and the processes with
several waits are testbench stimuli that run a few times. Interpreter instructions of the four: -0.004 %, -0.007 %, +0.12 %
(`grlib_leon3mp` 1688.42 to 1690.39 G, more copies optimised, none hot), +0.007 %; the other tests equal. It gains nothing, so it is not
landed.

(b) Folding the load of a pointer or handle register from instance data into a constant (the experiment replaced the load by its value in a
pointer register, which the interpreter accepts and the verifier would not, for the instruction count only): the optimised copies keep 0 to 2
such loads per process (`sc5_fifo_repack`: `p_col` 2 of 614 IR instructions, `p_pre` 2 of 124, the other seven processes of the design none), and the
interpreter instructions of the whole run change by -0.00 % (`sc5_fifo_repack`), +0.01 % (`poc_cache_cpu`), +0.01 % (`neorv32_single_min`),
-0.00 % (`hdlm_lfsr`), +0.02 % (`vai_pe_array`), -0.03 % (`hdlm_axi_fifo`), -0.14 % (`grlib_leon3mp`), -0.04 % (`fletcher_bufferwriter_prim32`),
+0.00 % (`fletcher_bufferwriter_512`). Descriptors and array bounds in instance data already fold (`const_dim`, the integer words of a
descriptor); what is left is the handle of a heap object, whose contents the optimiser cannot see, so a constant handle saves one load and
enables nothing. Not landed. The patches of both experiments are on #661.

The copies of designs without a stage-cache entry: `fletcher_bufferwriter_512` has none because the extent of its `Db` grows while it elaborates
(36 units at the image, 57 at the lowering, #671), `hdlm_lfsr` because it runs under a lowered library, whose closures have no entries of
their own (`library_made`; a `spec` entry per closure would take one of the 96 entries of a library directory for each long-running
testbench of a suite). The copies cost 72 times 0.7 M = 50 M instructions of 99 G in `fletcher_bufferwriter_512` each run (0.05 %), so a
`spec` entry outside the stage cache's key was not made; #671 would give it one.


## 13. What the declarative part leaves in the frame (#678, 2026-10-24)

Section 12 lists "folding the loads of pointer-typed instance data" as not worth it, because the body of a process reads its variables'
descriptors through frame slots the declarative part filled, not through instance data. The frame is the next thing the copy can know:
the copy takes over between two activations, and the declarative part is a function of the instance's data. `opt/decl.rs` runs it per
copy on an abstract machine and gives the walk the words the body never writes (`docs/decisions.md` "Facts the declarative part leaves
in a process copy", measurements in `docs/STATUS.md`). The copy of the LEON3 integer unit's `comb` loses all 551 `LayoutOf` and 653 `Field`
of the interpreter's version; the interpreter gains little on most designs (`fletcher_bufferwriter_512` -9.0 %, `grlib_leon3mp` -4.8 %
over the whole run) because it keeps a per-type layout memo and fuses `LayoutOf` with `Field`; the compiled tier, which answers each of
them with a callback or a compare of the descriptor bytes, is where the IR count is the figure that matters (the copies exist under
`--tier=auto`, not under `--tier=llvm`).

The same machine is the natural place for the evaluation of pure calls with constant arguments at specialisation time that the reference
simulator does (its elaboration folds calls of pure functions with constant arguments by running them), which `resize(INIT, q'length)` of
PoC's `ffdre` (#701) needs; it has no model of a call yet (an unknown result, the arguments' memory unknown for good).
