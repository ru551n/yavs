Open issues on the cost of a first run: library analysis throughput, cold elaboration, cache entries and the fixed per-run overhead for suites of many small tests. Decisions that gate the work are listed with it.

## Issues
- [ ] #747 perf: batched cold analysis is slower than sim1's batched call on small libraries (poc_strings, sc5_timestamp, neorv32_single_min)
- [ ] #745 decision: batch the files of a library into one yavs analyze call by default (cold analysis 26 to 50 percent fewer user instructions)
- [ ] #742 perf: cold library analysis throughput after #424 round 3 (per-file analyze, 13 M instructions per file)
- [ ] #665 perf: cold elab of the neorv32 testbench costs 5 percent more instructions with entries decoded one by one (0.634 G to 0.669 G)
- [ ] #539 perf: yavs elab of library testbenches is 3.4x sim1 cold, and a warm elab keeps re-evaluating constants (poc uart_rx 3x sim1)
- [ ] #558 decision: how a cold run should avoid cloning the bodies of generic package instances nothing calls (#555)
- [ ] #553 decision: lower library subprograms at analysis time so that a first elab links instead of lowering (cold elab 60 to 100 ms against 20 to 40 ms)
- [ ] #477 decision: parse package bodies of dependencies only when a body is needed (cold analyze of one file)
- [ ] #424 perf: cold analysis allocation and copying (parse, HIR construction, diagnostics bookkeeping)
- [ ] #426 perf: fixed per-run overhead (startup, image read, exit) for suites of many small tests
- [ ] #803 perf: a unit that loads a --relaxed-dependent unit shares one cache entry across both settings and is re-analysed in turn
- [ ] #799 perf: analysed-unit entries of files analysed by relative path are never read by elab or run
