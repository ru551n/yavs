Open issues on running yavs under the VUnit test runner and sharing CPUs across parallel simulations (jobserver-style token pool, see #481 item 7). Part of the M4 epic, #142.

## Issues
- [x] #865 vunit-yavs: test runner package with --yavs-debug, thread cap and jobserver (branch vunit-interface)
- [x] #870 vunit-yavs: native library hooks for VUnit packages that load a native library
- [ ] #881 A VUnit AXI-stream elementwise testbench times out under yavs only for 64-bit words and no stall
- [ ] #882 perf: --yavs-jobserver slows a single-thread processor-model test by about 4 percent
- [ ] #879 token server at large -p: cap threads per simulation by default and grow the thread pool only when tokens arrive
- [ ] #878 token server: fall back to one thread instead of failing when the socket cannot be used
- [ ] #801 investigate: VUnit hands free CPU slots to running yavs simulations in the tail of a -p N regression (jobserver-style token pool)
