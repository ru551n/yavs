Open issues in the test suite, CI jobs and benchmark harness: flaky or slow tests, platform-specific failures, and tools whose results depend on the host. Fix these before they hide real regressions.

## Issues
- [ ] #855 flaky test: spec.rs pre-key and copy cache counts on the Windows llvm job (a_warm_run_links_by_the_pre_key_and_the_check_agrees) (open: llvm-only test; the llvm CI job is gone (#898), nothing runs it now)
- [ ] #805 flaky: auto_tier_recompiles_hot_compiled_code_at_o2 'never recompiled' once in a full workspace run (open: llvm-only test; no CI job since #898)
- [x] #753 flaky test: sigint_ends_a_process_that_never_suspends timed out once under load
- [x] #690 flaky test: compile_thread_keeps_the_affinity_of_the_process reads comm of a thread that ended
- [ ] #649 flaky test: cli second_signal_exits_at_once aborts (exit 134, internal compiler error) in an llvm build under load (open: llvm-only build; no CI job since #898)
- [x] #631 tests: warm_files_are_verified_not_replayed fails on Windows (manifest paths with escaped backslashes are not normalised)
- [x] #852 tests: composite/aggregate_fill takes more than 300 s under the gc-stress debug build (any tier)
- [x] #843 editors/vscode: test/client.js exits 1 after all assertions pass (EPIPE at teardown)
- [x] #827 external.py --threads: three sim1 tests differ between thread counts by their environment (work directory, sub-second wall clock)
- [x] #806 difftest: yavs-auto-warm fails file_append_2019 (the second run appends to the first run's file)
- [x] #804 realbench: corpus libraries and timed cells use different std/ieee stores, so partial sessions miss std/ieee entries
- [x] #796 realbench: 300 s run timeout is hard-coded, grlib_leon3mp interp hits it on a loaded host
- [x] #624 difftest: external sim1 test issue1267 fails since the case-length check (#463, L120)
- [ ] #627 perf tooling: timed runs are disturbed by builds on the SMT siblings of the timing cores (open: harness cannot tell; the policy (timed runs on cores 8-15 only, siblings idle, `tools/claim core`) avoids it)
- [x] #780 tools: generated conformance report (docs/conformance.md), CI job and README evidence section
