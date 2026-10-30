Open issues on compiled code: tier policy and heat, compile cost, cache hits across warm runs, native-frame code, and the missing LLVM instruction coverage. Common theme: compiled code that is not faster than the interpreter, or whose compile cost is not repaid.

## Issues
- [x] #125 llvm: lower IR to LLVM 21 with tiering and background compilation
- [ ] #844 perf: Cranelift compilation cache misses across warm runs: a root's body depends on the run's compile tree
- [ ] #800 perf: --tier=auto charges the second compile of a function (other code kind) 10 to 20 times less than it costs
- [ ] #729 tier=auto with --tier-heat: native-frame code of a function stays at the first compile level (O0)
- [x] #728 decision: should a warm run price a function whose object is in the cache at the cache price (rtl_ff, lfsr64: 10 to 16 percent of the wall)
- [ ] #718 perf: --tier=clift-auto runs short verification-library testbenches at 1.2 to 1.5 times the interpreter's instructions (translated native variants gain nothing)
- [ ] #679 perf: compiled code of processes of 1 000 to 30 000 IR instructions is level with the interpreter, never faster (iu comb, state_register, exec_comb)
- [ ] #675 perf: --tier=auto never compiles the heavy combinational processes of neorv32 or the serial testbench process of hdlm_lfsr, and says nothing about it
- [ ] #673 perf: --tier=auto instruction counts of one binary vary by 10 to 80 percent between runs (the compile outcome depends on timing)
- [ ] #634 perf: compiled clocked RTL executes 20% fewer instructions than the interpreter and takes 5 to 15% more cycles (front end: branch, L1i and iTLB misses)
- [ ] #538 perf: --tier=llvm compiles the whole closure before time 0 (8 to 25 s on library testbenches) and a warm object cache still costs 1 to 3 s
- [ ] #532 perf: --tier=llvm on the neorv32 testbench: 14 to 17 s whole-program compile, and the run itself is slower than the interpreter (dual and cache configurations pass 300 s)
- [x] #432 llvm tier: compile cost per IR instruction, side-exit cost, and startup cost of the llvm build
- [ ] #740 perf: cold column against the reference simulator: where cold minus warm goes, compile cost of --tier=auto
- [ ] #341 llvm tier: real conversions, powers and physical-times-real
- [ ] #340 llvm tier: heap instructions (New, Deref, Free)
- [ ] #339 llvm tier: signal, driver and wait instructions as runtime calls
- [ ] #338 llvm tier: lower logic-array (bit-plane) instructions
- [ ] #337 llvm tier: natives, foreign calls and reports as direct runtime calls
