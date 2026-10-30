Open performance issues in the interpreter, the IR optimiser and the lowering of records, vectors, calls and large objects. Common theme: instruction cost per op, call and signal access, and memory footprint of large or per-process state.

## Issues
- [ ] #778 perf: SubtypeCheck costs 136 to 161 instructions an op in the interpreter (7 testbenches, 1.2 to 2.7 percent of the interpreted window)
- [ ] #771 perf: the fixed cost of one HRead call of the stdio package is 86 ops and 11 000 instructions for one digit
- [ ] #766 perf: the host layers around a compiled clocked process cost 600 to 700 instructions an activation (rtl_ff, rtl_lfsr, lfsr64)
- [ ] #757 perf: run phase of sc5_fifo_repack, poc_stat_histogram, sc5_timestamp vs sim1
- [ ] #730 perf: a call costs 50 to 80 ns in the interpreter and in frame-mode compiled code, 7 ns on a native frame
- [ ] #791 perf: interpreter execution of grlib_leon3mp and fletcher
- [ ] #790 perf: compiled drive and signal path of large processes (record and array signal reads, composite drives, lcopy staging)
- [x] #677 perf: a record signal costs about 680 instructions a leaf to read and to drive (GRLIB iu:comb is still 19 percent of the LEON3 run): gather and scatter instructions
- [ ] #678 optimiser: the specialised copy of GRLIB iu comb keeps 1527 LayoutOf and 1901 Field of 23641 IR instructions
- [ ] #669 perf: vector logic assignment is three unfused ops (WaveBuf, LCopy, Drive): 9 M per 300 us of neorv32
- [ ] #478 ir: dense register numbering or live-range sharing in the optimiser (frames average 255 registers in the library testbenches)
- [ ] #434 perf: per-process interpreter state is 520 bytes before the first activation (20000 instances: 22 MiB)
- [x] #430 interp: a preloaded zero register kept a stale value; verify that no register is read before it is written
- [ ] #520 perf: long interpreter-bound testbenches (poc arith_addw 6.7x, window generator 4.2x) against sim1; --tier=auto does not help
- [ ] #493 Test-runner queue push/pop costs about 32k instructions per character (protected-method calls): one testbench 15x slower than sim1
- [ ] #798 perf: whole-record assignments of 100s of KB (UVVM command record) need copy-on-write or sparse large objects
- [ ] #851 perf: a constrained vector variable of a process lives in a heap object, so the process never runs lean (lfsr64)
- [ ] #544 perf: --tier=auto costs about 15 instructions per activation when nothing is compiled (many tiny processes)
- [ ] #487 Large bit and boolean arrays are filled eagerly: 512 MiB bit_vector costs 537 MB RSS
