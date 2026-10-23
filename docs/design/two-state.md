# Speculative two-state compiled code: study and prototype (spike/2state)

Status: study with a working prototype on the branch `spike/2state` (not on main, off by default).
Tracking: #532 (compiled clocked RTL). Origin: another open-source Rust simulator (SystemVerilog)
lowers clocked and combinational blocks to a "two-state" form that assumes every value bit is 0 or
1, checks the assumption where the value is loaded, bails to the general path when it fails, and
backs off with doubling sleeps when bails repeat. yavs stores `std_ulogic` as nine values, four
bit-planes per 64 elements. Question: would compiling logic operations on the assumption of
`'0'`/`'1'` make compiled RTL smaller and faster, and what would it take?

## 1. Summary

1. The assumption holds. In compiled processes the guard of the prototype failed 0 times in 6 of
   the 8 head-to-head designs and in `sc5_fifo_repack`, `poc_arith_addw` and `hdlm_lfsr`, 27 times in
   5.9 M activations of `fletcher_bufferwriter_512` and 773 times in 16.6 M of `neorv32_single_min`
   (0.005 percent; the first cycles after power-up, before reset). Back-off puts 0.6 percent of the
   `neorv32_single_min` activations to sleep. Section 5.2.
2. The gain is small. The prototype speculates the signal read of 1 to 64 elements (`SigRead`), the
   one place where the kernel's code bytes can be turned into planes with one test. On the design
   where it matters most, `neorv32_single_min` (`--tier=llvm`, cold), it cuts the object from 20.7 MB
   to 20.3 MB (-2.1 percent), the compile from 106.7 G to 102.2 G instructions (-4.1 percent) and
   `init` from 11.3 s to 10.9 s (-3.1 percent). On the other designs of the set the object and compile
   time do not move (+-0.1 percent): few of their reads are two-state sites (`SigRead`s in process code of known length 1 to 64: `neorv32_single_min` 2387 of 2610, `fletcher_bufferwriter_512` 40 of 421, `poc_arith_addw` 4 of 136, `sc5_fifo_repack` 0 of 25, `hdlm_lfsr` 0 of 14; the others are wider, of run-time length or in subprograms).
   Run cycles are the same as without it within the noise of the host (section 5.3), also with the
   guard removed (`oracle`, the upper bound of this prototype).
3. The ceiling is low. Stepping every logic instruction in the interpreter (no code at all for
   `LCopy`, `SigRead`, `SigReadScalar`, `LGet`, `LSet`, `LFill`, `Cat`, `Drive`, `LogicOp`, `LNum`,
   `LEq` and every native) shrinks the object by only 16 to 27 percent (section 4.1), and compiled
   code is 9 to 39 percent of the cycles of a run (section 5.4). The code of compiled RTL is
   scaffolding (addresses, descriptors, layouts, checks, exits, host calls), not value
   representation: a two-state representation cannot remove it.
4. Correctness is cheap to keep: difftest 451 of 451 on the interpreter, `--tier=llvm` and
   `--tier=auto --tier-heat=1,0,0`, with the speculation on, in shadow mode (every passing guard
   compares the two-state planes with the general read and ends the run with status 70 on any
   difference) and with `--threads 4`; a test with all nine values at every position
   (`crates/yavs/tests/spec01.rs`).
5. Recommendation: do not land it. It is not clearly positive. The measured gain does not pay for a
   second semantics in the generators. If the maintainer wants the idea pursued, the form worth
   building is an IR-level known-01 fact (section 7), and the effort is better spent on the
   per-activation entry cost and the scaffolding of compiled code (#532, #339, the specialisation
   study).

## 2. What exists today

- Logic arrays are four bit-planes per block of 64 elements (`Block = [u64; 4]`, plane `k` is bit `k`
  of the `std_ulogic` code). A value that is `'0'`/`'1'` everywhere has planes `[bits, mask, 0, 0]`
  (codes 2 and 3).
- The kernel keeps, next to the planes, one code byte per element of every logic signal
  (`Signal::codes`, refreshed in `update_effective`); compiled code reads them in place
  (`TierSig::codes`, `Gen::sig_read_block`): eight codes at a time, four planes built from each
  group with a multiply-and-shift transposition. `SigReadScalar` is the byte itself.
- The scalar `std_logic_1164` gates are 81-entry tables (`yavs_interp::code_table`), inline in
  compiled code behind a range guard. The `ieee` natives already have a speculative one-word path
  (`ieee::small`: operands of up to 64 elements without a metavalue are read from the first block and
  computed in integers; anything else takes the general `Vh` path).
- Signal storage and resolution are plane based in the kernel (`update_word`: XOR of four planes,
  blend of four planes).

The reference simulator (sim1) keeps one byte per `std_ulogic` element and the gates are 81-entry
table lookups; it has no two-state fast path. Its lead over yavs on RTL is in folded widths and
indices, static layouts and call cost, not in the value representation (see
`docs/design/specialisation.md` on `spike/spec`). Idea used from it: nothing new for this study; it
confirms that nine-value tables are not the bottleneck.

## 3. Prototype

Switch: `YAVS_SPEC01=on|shadow|oracle` (default off; `YAVS_SPEC01_STATS=1` with `--time-report` prints
the counters; `YAVS_SPEC01_STRUCT=exit` below). Files:

| File | What |
|---|---|
| `crates/yavs-interp/src/spec01.rs` | Backend-neutral policy: `Mode`, the per-function back-off table, `bail`, `gate`, counters, `report`. No unsafe. The only thing a second backend must call is `bail(func, kind)` from its bail path and `gate(func, activation, run)` around its entry |
| `crates/yavs-llvm/src/lower.rs` | `Gen::sig_read_spec` (the two-state `SigRead`), `spec_bail`, `codes_word`; `YAVS_LLVM_KINDS` (LLVM instructions generated per IR instruction kind) and `YAVS_LLVM_ABLATE=Kind,Kind` (step those kinds in the interpreter, to measure what the generator spends on them); the gate in `LlvmTier::run`/`run_cached` and the same in `auto.rs` |
| `crates/yavs-interp/src/exec.rs` | `spec-census` feature (measurement builds): counts stepped logic reads that see only `'0'`/`'1'` |
| `crates/yavs/tests/spec01.rs` | Interpreter against `--tier=llvm` in modes off, on and shadow with every `std_ulogic` value at every position, non-vacuity of guard bails and back-off |

The two-state read (frame-mode code of a process; native-frame functions and run-time lengths are not
touched): for a plain reference of 1 to 64 elements to a logic signal that keeps its codes, load the
code bytes (the last group byte-exactly, so no allocation-tail guard), compute one violation word
`((w & 0xFE..FE) ^ 0x02..02) & valid` over the groups, one guard `violation == 0`, then the planes are
`[bits, mask, 0, 0]` with one multiply-and-shift for plane 0 instead of four. A failing guard is a
bail: the compiled function flushes the registers live at the `SigRead` (the existing `flush_exit`)
and returns its pc, the interpreter executes the `SigRead` and goes on. A signal compiled code does
not read in place (not in the table, range outside, destination not reachable; "structural") takes
the general read through the interpreter and rejoins, as before; with `YAVS_SPEC01_STRUCT=exit` it
bails too, which lets LLVM see the stored constant planes downstream. Measured on `neorv32_single_min`: the
object is 1.3 percent larger than with the join (the exits cost code too), and 3.3 percent
(`neorv32_single_min`) and 7.8 percent (`fletcher_bufferwriter_512`) of the activations go to sleep
for structural bails, so it is not the default.

Back-off (`spec01::bail`, `gate`): a bail puts the function to sleep for `2^k - 1` activations (`k`
counts up from 1 to 10 with repeated bails); the interpreter runs the sleeping activations; after the
sleep one retry (probation) runs compiled; a retry without a bail resets `k` and disarms the
function. A function that never bailed costs one atomic load and a test per activation (the table of
slots is only looked at when the speculation is on). An entry in the middle of an activation (the
interpreter hands a frame to compiled code after a jump, `Tier::run`) neither uses up the sleep nor
decides a retry (`activation = false`); only `Tier::run_cached`, the process activation, does.

Shadow mode (`YAVS_SPEC01=shadow`): after the two-state read stored its planes, the same
instruction is executed by the interpreter into the same memory and the four planes are compared; a
difference prints an internal error naming function and pc and exits with status 70. It checks the
whole claim "guard passes implies the two-state result is the general result", including the
agreement of the code bytes with the planes.

Oracle mode (`YAVS_SPEC01=oracle`): no guard. Wrong on any non-01 value (it changes the output of
`neorv32_single_min`, which reads metavalues at time 0); it is the upper bound of what this
prototype could save if its guard were free.

## 4. Bounds, before measuring

### 4.1 What the generator spends on logic instructions

Cold `--tier=llvm` object size (bytes of the object in the compilation cache) with the named
instruction kinds stepped by the interpreter instead of compiled (`YAVS_LLVM_ABLATE`), against the
normal build; change in percent. The all-logic row also steps `CallNative` (all natives, not only
the logic ones), `LogicOp`, `LNum` and `LEq`.

| stepped kinds | poc_arith_addw | fletcher_bufferwriter_512 | neorv32_single_min | sc5_fifo_repack | hdlm_lfsr |
|---|---:|---:|---:|---:|---:|
| `LCopy` | -1.5 | -12.8 | -4.2 | -2.0 | -1.2 |
| `SigRead`, `SigReadScalar` | -1.5 | -3.4 | -11.0 | -2.7 | -0.1 |
| `LGet`, `LSet`, `LFill` | -1.4 | -4.4 | -3.9 | -7.2 | -2.0 |
| `Cat`, `Drive` | -7.7 | -4.8 | -6.3 | -3.4 | -2.9 |
| `CallNative` (all natives) | -10.2 | -6.4 | -1.5 | -4.1 | -11.4 |
| all of the above and `LogicOp`, `LNum`, `LEq` | -17.0 | -23.5 | -26.8 | -16.4 | -15.9 |

Two-state code cannot go to zero: a guard, the exit and the remaining planes stay. So the
object-size ceiling of the idea over the whole logic family is well below the last row.

LLVM instructions per IR instruction kind (`YAVS_LLVM_KINDS`, before the pass pipeline), share of the
module, `neorv32_single_min`: `Alloc` 13.1 percent (36 per IR instruction), `SigRead` 11.7 (116),
`LCopy` 10.7 (175), `SigVal` 7.9, `Store` 6.4, `Load` 5.8, `Index` 5.7, `Drive` 5.4, `LSet` 5.0, `Copy`
4.4, `Stmt` 4.3, `SigReadScalar` 2.8. In `fletcher_bufferwriter_512`: `Load` 13.1, `LCopy` 9.6 (236
per instruction), `Alloc` 7.4, `LayoutOf` 7.0, `Store` 7.0, `Field` 6.9, `DopeLen` 6.7, `SubtypeCheck`
5.3. In `poc_arith_addw`: `Load` 19.8, `LayoutOf` 14.1, `Copy` 7.5, `Alloc` 6.6. The bulk is address
resolution, descriptors and checks, which two-state does not touch.

### 4.2 Where the time is

Share of the samples in compiled code (`[JIT]` rows of `perf report --sort dso`, `--tier=llvm`,
whole run), the rest is the yavs binary (kernel, callbacks, entry/exit):

| design | compiled code | yavs binary |
|---|---:|---:|
| h2h_rtl_ff | 19.1 | 79.0 |
| h2h_rtl_lfsr | 25.9 | 69.9 |
| h2h_lfsr64 | 21.8 | 72.1 |
| h2h_lfsr64_xor | 9.1 | 34.9 |
| sc5_fifo_repack | 35.1 | 60.0 |
| hdlm_lfsr | 9.2 | 8.6 |
| fletcher_bufferwriter_512 | 30.4 | 42.4 |
| neorv32_single_min | 22.0 | 30.1 |
| poc_arith_addw | 39.1 | 47.1 |

(The remainder is the dynamic loader, the kernel's libc calls and the compile thread.) In
`h2h_rtl_lfsr` the top of the yavs share is `enter`, `run_quick`, `view_again`, `run_batch` and
`finish_direct`: entry cost and the kernel, not value operations. Even a free logic operation can
only save a part of the compiled share.

## 5. Results

Host: Ryzen 9 7950X3D, loaded by other agents (load average 20 to 22 throughout). Timed runs pinned to
core 12 or 13 (CCD1), interleaved, warm compilation cache, 5 to 7 runs, medians; builds with
`tools/build-bench.sh --features llvm`, the baseline built from the commit the branch starts from.
Cycles and instructions are user-mode (`perf stat -e cycles:u,instructions:u`). The noise is large:
the same source with a different inlining decision moved the instruction count of `h2h_rtl_ff` by
+1.4 percent with the speculation off and by up to +3.5 percent with an empty gate, and single
cells moved the cycles by 2 to 4 percent between identical binaries. Differences under 3 percent
are not results.

### 5.1 Code size and compile time (cold, `--tier=llvm`, stopped at 0 fs, 5 runs)

| design | cell | object KB | vs base | init s | vs base | compile instr G | vs base |
|---|---|---:|---:|---:|---:|---:|---:|
| sc5_fifo_repack | base | 391 |  +0.0 | 0.93 |  +0.0 | 8.98 |  +0.0 |
| sc5_fifo_repack | off | 391 |  +0.0 | 0.93 |  -0.2 | 8.98 |  +0.0 |
| sc5_fifo_repack | on | 391 |  +0.0 | 0.93 |  -0.2 | 8.99 |  +0.1 |
| sc5_fifo_repack | oracle | 391 |  +0.0 | 0.92 |  -1.1 | 8.98 |  +0.0 |
| poc_arith_addw | base | 4542 |  +0.0 | 3.24 |  +0.0 | 34.75 |  +0.0 |
| poc_arith_addw | off | 4542 |  +0.0 | 3.22 |  -0.6 | 34.71 |  -0.1 |
| poc_arith_addw | on | 4542 |  -0.0 | 3.22 |  -0.6 | 34.69 |  -0.1 |
| poc_arith_addw | oracle | 4541 |  -0.0 | 3.19 |  -1.7 | 34.75 |  +0.0 |
| fletcher_bufferwriter_512 | base | 7870 |  +0.0 | 5.07 |  +0.0 | 51.25 |  +0.0 |
| fletcher_bufferwriter_512 | off | 7870 |  +0.0 | 5.10 |  +0.6 | 51.29 |  +0.1 |
| fletcher_bufferwriter_512 | on | 7865 |  -0.1 | 5.17 |  +1.9 | 51.16 |  -0.2 |
| fletcher_bufferwriter_512 | oracle | 7860 |  -0.1 | 5.13 |  +1.1 | 51.22 |  -0.1 |
| neorv32_single_min | base | 20739 |  +0.0 | 11.26 |  +0.0 | 106.65 |  +0.0 |
| neorv32_single_min | off | 20739 |  +0.0 | 11.49 |  +2.1 | 106.71 |  +0.1 |
| neorv32_single_min | on | 20300 |  -2.1 | 10.90 |  -3.1 | 102.24 |  -4.1 |
| neorv32_single_min | oracle | 19884 |  -4.1 | 10.61 |  -5.7 | 100.20 |  -6.0 |
| hdlm_lfsr | base | 11034 |  +0.0 | 18.97 |  +0.0 | 211.69 |  +0.0 |
| hdlm_lfsr | off | 11034 |  +0.0 | 18.95 |  -0.1 | 211.27 |  -0.2 |
| hdlm_lfsr | on | 11034 |  +0.0 | 18.91 |  -0.3 | 211.86 |  +0.1 |
| hdlm_lfsr | oracle | 11034 |  +0.0 | 18.92 |  -0.3 | 211.62 |  -0.0 |
| h2h_rtl_lfsr | base | 33 |  +0.0 | 0.07 |  +0.0 | 0.67 |  +0.0 |
| h2h_rtl_lfsr | off | 33 |  +0.0 | 0.07 |  -1.4 | 0.67 |  +0.1 |
| h2h_rtl_lfsr | on | 33 |  +0.3 | 0.07 |  -2.7 | 0.66 |  -1.3 |
| h2h_rtl_lfsr | oracle | 33 |  -1.1 | 0.07 |  -4.1 | 0.65 |  -2.3 |
| h2h_rtl_ff | base | 23 |  +0.0 | 0.06 |  +0.0 | 0.56 |  +0.0 |
| h2h_rtl_ff | off | 23 |  +0.0 | 0.06 |  -1.8 | 0.56 |  +0.0 |
| h2h_rtl_ff | on | 23 |  +0.0 | 0.06 |  +0.0 | 0.56 |  +0.1 |
| h2h_rtl_ff | oracle | 23 |  +0.0 | 0.06 |  -1.8 | 0.56 |  +0.1 |

(Percent against `base`.)

`off` is the same binary with the speculation off (the noise floor of the build); `on` is the
guarded two-state read; `oracle` has no guard.

### 5.2 Bail rates (compiled processes, `--tier=llvm`; `--tier=auto --tier-heat=1,0,0` agrees)

| design | activations | guard bails | functions that bailed | activations asleep |
|---|---:|---:|---:|---:|
| h2h_rtl_ff | 8204098 | 0 | 0 | 0 (0.00%) |
| h2h_rtl_lfsr | 2580514 | 0 | 0 | 0 (0.00%) |
| h2h_rtl_chain | 2006003 | 0 | 0 | 0 (0.00%) |
| h2h_rtl_fan | 20024002 | 0 | 0 | 0 (0.00%) |
| h2h_rtl_bus | 720034 | 0 | 0 | 0 (0.00%) |
| h2h_rtl_ram | 100003 | 0 | 0 | 0 (0.00%) |
| h2h_lfsr64_xor | 335066 | 1 | 1 | 1 (0.00%) |
| h2h_lfsr64 | 1340066 | 1 | 1 | 1 (0.00%) |
| sc5_fifo_repack | 2295669 | 0 | 0 | 0 (0.00%) |
| poc_arith_addw | 38133434 | 0 | 0 | 0 (0.00%) |
| fletcher_bufferwriter_512 | 5886197 | 27 | 9 | 147 (0.00%) |
| neorv32_single_min | 16638551 | 773 | 192 | 101049 (0.61%) |
| hdlm_lfsr | 1048578 | 0 | 0 | 0 (0.00%) |

Structural bails are not counted (they take the general read and join). Interpreter census
(`spec-census` build, stepped `SigRead`/`SigReadScalar` only; the fused ops of the fast loop bypass
it, so the h2h designs show 0): vector reads that see a non-01 value: `poc_arith_addw` 0.03 percent,
`fletcher_bufferwriter_512` 0.9, `neorv32_single_min` 6.7 (power-up values and a testbench that
prints them), `hdlm_lfsr` 0.008; scalar reads: `neorv32_single_min` 61 of 1.1 M. The bail counters of
the compiled processes are the better figure.

### 5.3 Run cycles and IPC

Ratios to the baseline build (below 1 is faster). `off`, `on`, `oracle` are the same binary.

| design | tier | core | runs | base cycles G | base IPC | off cyc | on cyc | oracle cyc | off instr | on instr | oracle instr |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| h2h_rtl_ff | auto | 12 | 7 | 3.70 | 3.32 | 1.001 | 1.027 | 1.040 | 1.017 | 1.025 | 1.014 |
| h2h_rtl_ff | llvm | 12 | 7 | 3.59 | 3.40 | 0.998 | 1.003 | 1.020 | 1.014 | 1.024 | 1.014 |
| h2h_rtl_lfsr | auto | 13 | 7 | 1.69 | 3.42 | 1.015 | 0.998 | 0.983 | 1.006 | 1.001 | 0.990 |
| h2h_rtl_lfsr | llvm | 13 | 7 | 1.64 | 3.64 | 0.996 | 0.990 | 1.020 | 1.009 | 1.007 | 0.995 |
| h2h_lfsr64 | auto | 13 | 7 | 0.97 | 3.40 | 0.982 | 0.953 | 0.999 | 1.016 | 1.023 | 1.016 |
| h2h_lfsr64 | llvm | 13 | 7 | 0.84 | 3.68 | 1.006 | 0.987 | 0.984 | 1.009 | 1.015 | 1.009 |
| h2h_lfsr64_xor | auto | 12 | 7 | 0.69 | 2.31 | 1.006 | 1.004 | 0.990 | 1.000 | 1.003 | 1.003 |
| h2h_lfsr64_xor | llvm | 12 | 7 | 0.61 | 2.26 | 0.969 | 0.951 | 0.598 | 1.005 | 1.002 | 0.996 |
| h2h_rtl_chain | auto | 12 | 7 | 0.97 | 3.39 | 0.966 | 0.965 | 0.952 | 1.000 | 1.000 | 1.000 |
| h2h_rtl_chain | llvm | 12 | 7 | 1.33 | 2.93 | 0.876 | 0.869 | 0.872 | 1.011 | 1.018 | 1.011 |
| h2h_rtl_fan | auto | 13 | 7 | 4.70 | 3.11 | 0.996 | 0.991 | 0.994 | 1.000 | 1.000 | 1.000 |
| h2h_rtl_fan | llvm | 13 | 7 | 5.97 | 3.08 | 0.996 | 0.998 | 0.996 | 1.023 | 1.039 | 1.023 |
| h2h_rtl_bus | auto | 13 | 7 | 0.99 | 3.83 | 0.999 | 1.008 | 1.002 | 1.000 | 1.000 | 1.000 |
| h2h_rtl_bus | llvm | 13 | 7 | 1.10 | 3.71 | 0.994 | 0.999 | 0.993 | 1.004 | 1.006 | 1.004 |
| h2h_rtl_ram | auto | 12 | 7 | 0.34 | 1.81 | 1.000 | 0.991 | 1.006 | 1.000 | 1.000 | 1.000 |
| h2h_rtl_ram | llvm | 12 | 7 | 0.48 | 1.82 | 1.025 | 1.002 | 1.120 | 1.002 | 0.997 | 0.992 |
| sc5_fifo_repack | auto | 12 | 7 | 17.34 | 3.51 | 1.010 | 1.013 | 1.004 | 1.001 | 1.001 | 1.001 |
| sc5_fifo_repack | llvm | 12 | 7 | 12.37 | 3.52 | 1.006 | 1.016 | 1.012 | 1.001 | 1.002 | 1.001 |
| hdlm_lfsr | auto | 13 | 5 | 10.88 | 3.43 | 0.990 | 0.998 | 0.988 | 1.000 | 1.000 | 1.000 |
| hdlm_lfsr | llvm | 13 | 5 | 21.57 | 2.96 | 1.003 | 0.997 | 1.007 | 1.001 | 1.002 | 1.001 |
| fletcher_bufferwriter_512 | auto | 12 | 5 | 51.62 | 2.48 | 0.997 | 0.998 | 1.002 | 1.000 | 1.003 | 1.003 |
| fletcher_bufferwriter_512 | llvm | 12 | 5 | 86.04 | 1.64 | 1.019 | 1.024 | 1.003 | 1.002 | 1.002 | 1.000 |
| poc_arith_addw | auto | 12 | 5 | 109.06 | 3.51 | 1.022 | 1.012 | 1.015 | 1.002 | 1.003 | 1.002 |
| poc_arith_addw | llvm | 12 | 5 | 119.24 | 3.00 | 1.010 | 1.019 | 1.009 | 1.002 | 1.000 | 0.997 |
| neorv32_single_min | auto | 13 | 5 | 39.46 | 2.43 | 1.019 | 1.041 | 1.018 | 1.000 | 1.000 | 1.000 |
| neorv32_single_min | llvm | 13 | 5 | 69.57 | 1.54 | 1.005 | 1.005 | diverges | 1.003 | 0.991 | diverges |

Reading: instructions are within 3 percent everywhere (the processes that bail have a few hundred
activations in the interpreter), cycles are within the noise. `oracle` on `neorv32_single_min` under
`--tier=llvm` diverges (it is wrong on its metavalues) and its numbers are not used.

### 5.4 Other measurements

- The kernel's own `refresh_block_codes` (planes to code bytes, run on every change of a signal) with
  a two-plane path when planes 2 and 3 are zero (U, X, 0, 1 only): no measurable effect on
  `fletcher_bufferwriter_512`, `neorv32_single_min`, `sc5_fifo_repack`, `h2h_rtl_ff`, `h2h_rtl_ram`
  (`--tier=interp`: instructions +0.0 to +0.2 percent, cycles -0.5 to +1.5 percent, noise). Not kept
  (kernel code of another owner).

## 6. Correctness

- difftest (451 tests), speculation on: interpreter (modes shadow), `--tier=llvm` modes on, shadow,
  shadow with `YAVS_SPEC01_STRUCT=exit`, shadow with `--threads 4`, `--tier=auto --tier-heat=1,0,0`
  modes on and shadow: 451 of 451 each, the same as the baseline.
- `crates/yavs/tests/spec01.rs`: all nine values at every position of vectors of 1, 7, 9, 33, 64
  elements and slices of odd offset and length, read by compiled processes; output equal to the
  interpreter in modes off, on, shadow and with structural exits; guard bails and back-off sleep
  are asserted. Mutation check: the oracle build gives a different output on the same test.
- Back-off and bails cannot change results: both paths compute the same values; only the speed
  differs. Output is identical at any thread count (the policy state is relaxed atomics; a race
  moves a sleep, never a value).

## 7. Cranelift fit and the IR-level form

What the prototype shares with a second backend: `spec01.rs` (modes, back-off, counters), the exit
protocol (every backend already has "leave to the interpreter at pc with the live registers
flushed"), the kernel's code bytes and `TierSig`, the shadow idea (compare against the interpreter's
step). What is LLVM specific: about 150 lines in `Gen` (the byte transposition and the guard).
So the form is cheap to port, but it is also small because it only covers the one instruction.

A form worth building if the idea is pursued, at the IR level so every backend and the optimiser see
it: a known-01 fact per logic array register (a lattice value next to the existing constants and
address facts in `yavs-ir`'s optimiser) established by a guard instruction at the sources (a
`SigRead` or a load of a variable of unknown content: `Assume01 {addr, n}` that bails when a plane
bit says otherwise, or reads a kernel-maintained "has non-01" bit per plane block, which is cheaper
than the plane test) and consumed by rewrites: `LogicOp`, `LEq`, `LNum`, `LGet` on known-01 operands
become one-word `Bin`/`Cmp` instructions on a plane-0 word, `LCopy` of known-01 data copies one
plane and writes the constant planes. That is where a second, smaller semantics would pay (fewer
planes everywhere downstream), at the price of the optimiser pass, a deopt story for the guard (the
existing exits are per instruction and flush live registers, which is enough), and keeping the
interpreter the reference (the guard is a no-op there). The measurements above say that the pay-off
is bounded by the 16 to 27 percent of the object that the whole logic family is, minus the guards.

## 8. Questions for the maintainer

1. Is a bounded gain (code and compile time -2 to -4 percent on one design, nothing else measurable)
   worth a second, speculative semantics in the generators? Proposed: no.
2. Should the IR-level known-01 form of section 7 be tried at all, given the ceilings of section 4?
   If yes, it needs the optimiser owner and a kernel "has non-01" bit (kernel owner).
3. Should the effort go to per-activation entry cost (`enter`, `view_again`, `run_quick` are the top
   of the yavs share in section 4.2) and the scaffolding of compiled code (#532, #339, instance
   specialisation) instead?

## 9. Reproducing

```
git checkout spike/2state
tools/build-bench.sh --features llvm           # then, in a design's work directory:
YAVS_SPEC01=on YAVS_SPEC01_STATS=1 yavs run --tier=llvm --time-report TOP
YAVS_SPEC01=shadow yavs run --tier=llvm TOP     # internal error, status 70, on a mismatch
YAVS_LLVM_KINDS=1 yavs run --tier=llvm --stop-time=0fs TOP   # LLVM instructions per IR kind
YAVS_LLVM_ABLATE=LCopy,SigRead yavs run --tier=llvm --stop-time=0fs TOP
cargo build --release -p yavs --features "llvm yavs-interp/spec-census"   # census build
cargo test -p yavs --features llvm --test spec01
```
