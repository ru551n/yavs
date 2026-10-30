Open performance issues in the kernel cycle loop, drivers, commits, parallel update and process classification. Common theme: per-event and per-drive instruction cost, and processes classified serial when they could run lean or in parallel.

## Issues
- [x] #420 perf: kernel fast paths still open after #385 (skip unchanged assignments, static sensitivity, resolution tables, more edge-trigger shapes)
- [ ] #676 perf: single-element drives of a vector signal in a loop (state(i) <= x) are one commit entry each: 21 percent of hdlm_lfsr
- [ ] #734 perf: finishing a changed word of a narrow signal costs about 410 instructions (finish_direct 7% of neorv32_single_min): stamps, run lists and codes
- [ ] #795 perf: kernel cycle loop of small clocked RTL (cycle, finish_direct, assign_codes_now, commit_codes) against sim1
- [ ] #802 perf: a staging buffer per direct driver instead of one list entry per drive (drive and commit side of loops over vector elements)
- [ ] #859 perf: parallel update of one wide signal by element range (#481 item 3, step 3)
- [ ] #858 perf: --threads N costs designs that never pool 1 to 2 percent instructions and 0.94x to 0.98x wall
- [ ] #519 perf: wide-vector testbenches (512 bit bus, stream repacker) are 7x to 12x slower than sim1: interpreter and kernel signal path
- [ ] #504 A process that only writes to OUTPUT is classified serial
- [ ] #503 A process that calls a method of a protected object local to the process is classified serial
- [ ] #592 perf: conditional clock generators (clk <= not clk after T when running) are interpreted processes
- [ ] #724 perf: serial processes whose shared-state paths are rare still pay the cold activation path at every wake
- [ ] #720 perf: grlib_leon3mp whole-run profile: cold path of serial processes, port propagation, same-value drives
- [ ] #647 perf: clock-edge idioms not yet one edge instruction: 'event and level and 'last_value, wait until on an element, 'stable(0 ns)
- [ ] #788 perf: next glue-logic shapes after #786 run as interpreted processes (expression trees, compound and multi-arm conditions)
