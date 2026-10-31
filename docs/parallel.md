# Parallel simulation

The user guide has the page for users: [Threads and the token server](https://yavs.readthedocs.io/en/latest/threads.html).
This document holds the behaviour in detail and the measurements behind it, for developers.

`yavs run --threads N` lets one simulation use N CPU threads (default 1, no thread pool). Only
the evaluation of VHDL processes is spread over the threads, and only when many processes wake at
the same moment. Large, flat RTL designs, where thousands of processes wake on the same clock edge,
run up to about 2.5 times faster on 4 threads and 3.5 times on 8. Testbench-style designs and
processor models run as fast as on one thread: yavs then keeps the work on one thread. Leave it at
1 for regressions that already run tests side by side, and measure before you rely on it. Under a
token server named in `YAVS_TOKENS`, several simulations share one CPU budget (see "Thread budget
across parallel test runs" below).

## What it does

| Topic | Behaviour |
|---|---|
| What runs in parallel | Only process evaluation. Parsing, analysis, elaboration and the rest of a simulation step are single-threaded. |
| When a group of processes goes to the threads | A "batch" is the set of processes that wake in one delta cycle. A batch goes to the threads only when its estimated work is worth the hand-over (from about 20 microseconds of work that can be split); a smaller one runs on the main thread, as with `--threads 1`. The estimate comes from timing a sample of each process's runs on the main thread, and the threshold rises for a design whose batches turn out no faster on the threads (processes with much state, such as a processor's pipeline). |
| How a batch is shared | The batch is cut into pieces of neighbouring processes that the worker threads and the main thread take in turn; `--threads N` means N busy threads (N - 1 workers and the main thread). A clocked process that only drives signals is committed by the thread that ran it. |
| Processes that stay serial | A process that touches files, shared variables or protected objects, or calls something that does, runs on the main thread. In a batch that goes to the threads the others run first, then the serial ones one after the other, in the same order as on one thread. |
| Signal updates | Spread over the threads only when at least 256 independent signals change in one delta cycle. |
| Results | Output, reports, waves, coverage and exit status are byte-identical at any thread count, and with `--shuffle` for the same seed. The test corpus is checked at 1 thread, 4 threads and 4 shuffled threads. |
| Checks | Kernel tests compare 1 and 8 threads and several shuffle seeds, and the CI run of the test corpus at 4 threads compares 1 and 4 threads. |
| Compile thread | If a compiled tier is built in, `--tier=auto` compiles hot functions on one more background thread (see [install](install.rst)); it is not counted in `--threads`. |
| Debugging | `--threads` is ignored under `--dap` (warning `W0109`), so a debug session behaves the same every time. |

## Measured speedup

Measured 2026-10-28 on `main`: speedup over one thread of the same build, whole run (simulation phase in
parentheses), interpreter, median of 5 rounds that interleave thread counts, each run pinned to as many cores as
threads. The designs are small RTL benchmarks (`tools/bench`) and the testbench of a RISC-V soft-core processor.

| Design | 2 threads | 4 threads | 8 threads |
|---|---|---|---|
| rtl_fan: 4000 small processes on one clock | 1.61x (1.70x) | 2.39x (2.63x) | 3.25x (3.89x) |
| rtl_ff: 4096 one-bit registers | 1.33x (1.37x) | 1.93x (2.10x) | 2.64x (3.04x) |
| rtl_lfsr: 512 16-bit registers | 1.33x (1.41x) | 1.69x (1.86x) | 2.07x (2.41x) |
| 64 registers, a resolved bus, a chain of one-process stages | 0.95x to 1.07x | 0.96x to 1.02x | 0.95x to 1.01x |
| Soft-core processor, one or two cores | 0.94x to 1.00x | 0.96x to 0.98x | 0.97x to 0.98x |

Before 2026-10-28 the mid-size designs ran at 0.35x to 0.6x on several threads and the soft core at 0.6x to 0.75x;
flat RTL gained at most 1.6x.

## Planned

* **Parallel signal update.** On flat RTL the main thread still spends about 15 percent of a
  4-thread run committing and updating the signals the processes drove; splitting that work by
  element range is the next candidate ([#481](https://github.com/ru551n/yavs/issues/481)).

## Thread budget across parallel test runs

When yavs runs with `--threads N` above 1 and the environment variable `YAVS_TOKENS` names a
token server (`<port>:<secret>`: a server on 127.0.0.1 that the test runner starts, the same on
Linux and Windows), the threads come from that server
([#801](https://github.com/ru551n/yavs/issues/801)):

* `--threads N` is the cap. Each simulation owns one implicit token, its main thread, and can use
  at most N - 1 more lent by the server. The tokens decide how many of the N threads run at a time.
  The worker threads are started when the run is lent its first token (#879), so a simulation that
  never gets one has no more threads than a run with `--threads 1`.
* Tokens are asked for only when a group of processes is worth handing to the threads, never by
  waiting: while none is lent, the group runs on the main thread, as with `--threads 1`.
* Tokens are held while groups keep going to the threads and given back after 64 groups in a row
  that stayed on the main thread. The connection to the server closes when the simulation ends in
  any way (normally, with an error, on Ctrl-C, killed or crashed), and that gives every token back.
* Output, reports, waves, coverage and exit status do not depend on the tokens: they are the
  same at any token count, as at any thread count.
* Without `YAVS_TOKENS`, or with `--threads 1` (the default), it is not read and `--threads`
  behaves as before. `--dap` runs on one thread and ignores it.
* A `YAVS_TOKENS` yavs cannot use (not `<port>:<secret>`, or a secret the server does not accept,
  often a value left over from a test runner that ended) is error `E0128` before analysis (exit 2).
  Unset it to run without a token server. A server that cannot be reached, because nothing listens,
  it does not answer or the connection breaks during the run, is warning `W0118`: the run goes on
  with one thread and gives the same output.
* Without a token server, nothing shares the CPU between simulations run side by side: do not
  combine a high `-p` with a high `--threads`, the threads of all runs add up.

The protocol is in `docs/cli-contract.md` ("Token server") in the source tree.

A test runner that runs `-p N` simulations side by side can hand each of them `--threads N` and
lend the slots no test uses as tokens: one more each time one of its N slots runs out of tests. The long tests at the end of a regression then use the cores that are
free. This pays only where more threads speed a simulation up: flat RTL designs (see the measured
speedup above). Processors and testbench-style designs keep their batches on one thread and gain
nothing from extra tokens. A compiled tier's compile thread (`--tier=auto`) is not counted.

## Token server, measured

Measured 2026-10-29 on a regression of 24 testbenches from a downstream FPGA project (2 to 4 per
module, 0.1 s to 3.7 minutes each: AXI and Ethernet blocks, a soft-core processor, a CNN
accelerator, board-level system benches), `python run.py -p 8` with and without
`--yavs-jobserver`, yavs 0.11.0, the whole run pinned to 8 cores. Median of 3 runs that alternate
the two modes, after one untimed warm-up run of each. All 24 tests pass in both modes. The
"tail" is the time from the first moment, once the last test has started, that fewer than 8
simulations run, to the end of the last one.

| | without | with `--yavs-jobserver` |
|---|---|---|
| Total wall | 225.7 s | 234.4 s (+3.9 %) |
| Tail | 194.8 s | 201.4 s (+3.4 %) |

The token server gained nothing here, and cost about 4 percent. The run is decided by one test,
a soft-core processor benchmark that takes 222 s alone on one thread; it keeps all its work on
one thread (CPU time equals wall time), so the 7 other cores that sat idle during the tail were
never used. It ran 4 percent longer with the server (230 s against 222 s), probably because
other tests borrowed threads while all 8 slots were busy. Of the 24 tests only three used more
than one thread on average (a CNN-accelerator system bench 1.8, a board boot bench 1.8, a
firmware bench 1.3); they ended between 4.6 s earlier and 4.7 s later than without the server
and none of them is on the critical path. The rest ran at 0.9 to 1.1 threads. The server pays
only when the long tail is a flat RTL design (see "Measured speedup"); a regression whose long
tests are processors or testbenches does not gain, and with the server the processors cost a
little more than without.

A single large test run alone at `-p 8`, so that it can borrow up to 7 tokens (same host and method,
median of 3 interleaved runs after an untimed warm-up of each mode, simulation wall time and CPU time of the
`yavs run` process):

| Test | Mode | Wall | CPU | Average threads | Change in wall |
|---|---|---|---|---|---|
| CNN accelerator system bench | without | 47.7 s | 48.9 s | 1.02 | |
| | with `--yavs-jobserver` | 36.0 s | 77.4 s | 2.18 | 1.33x faster |
| Soft-core processor benchmark | without | 219.9 s | 221.2 s | 1.01 | |
| | with `--yavs-jobserver` | 224.7 s | 226.1 s | 1.01 | 2.2 % slower |

The accelerator bench pools: it gains a third with 2.2 threads on average, and the extra threads cost
58 percent more CPU time for that third. The processor model never uses a token, and the token mode alone
costs it 2 percent even with seven cores free ([#882](https://github.com/ru551n/yavs/issues/882)).

## Details

* **Why mid-size batches no longer slow down.** Handing a batch to the threads has a fixed cost of
  a few microseconds and moves the processes' data between cores. A batch of a few dozen cheap
  processes costs less than that on one thread, so it stays there.
* **Why processors and testbench-style designs do not gain.** A processor's pipeline processes
  carry much state that another core's caches lack, so their batches run no faster on several
  threads, and yavs learns that and keeps them on one thread. Testbench-style designs spend most
  of their process time in processes that must stay serial (files, shared variables, protected
  objects). Their levers are single-thread speed, and a token server gives them no extra speed.
* **What it costs when nothing goes to the threads.** About 1 to 2 percent more instructions; the
  design runs at about 0.95x to 1.0x of one thread.
* Full measurements: "Parallel kernel, step 1" and "step 2" in `docs/STATUS.md`; the design is
  `docs/design/parallel-kernel.md` in the source tree.
