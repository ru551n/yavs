Open issues in the command line, the DAP server, coverage reports, profiling and waveform recording. Common theme: exit codes, message formats and session behaviour that scripts and editors depend on.

## Issues
- [ ] #869 `yavs run` silently re-reads sources with its own `--encoding`, not the one the units were analysed with
- [ ] #839 cli: --message-format=json with a multi-file analyze stop prints plain text lines after the JSON; --threads=0 and --stop-time=10 messages; finish(2)/finish(3) exit codes clash with usage/internal error
- [ ] #840 tools: test-runner-requirements per-file analyzed report is not implemented; -Werror=CODE first label still says warning
- [ ] #864 dap: over TCP the session is not closed after `terminated` when the run ends with an internal error
- [ ] #863 dap: E9999 "read of 8 bytes at 0 outside a frame object of 0 bytes" in a test runner testbench under --dap
- [x] #696 YAVS_COUNTERS prints nothing in a --dap session
- [x] #877 cov report: readable text output (relative paths, named branch arms, one toggle wording) and --total for the coverage percentage only
- [ ] #883 Coverage: inline condition-combination exclusions with shorthand and default validation
- [ ] #876 run: drive the Surfer waveform viewer over WCP (--surfer): plan
- [x] #587 profile: source lines of compiled code, profile on Ctrl-C (follow-up of #580)
- [ ] #571 perf: waveform recording, what is left after #410 round 2 (writer thread, library per-change cost, file size)
- [ ] #364 dap: function calls and attributes in breakpoint conditions; force and deposit of signal parts
