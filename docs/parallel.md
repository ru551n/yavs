# Parallel simulation

`yavs run --threads N` lets one simulation use N CPU threads (default 1, no thread pool). Only
the evaluation of VHDL processes is spread over the threads, and only when many processes wake at
the same moment. Large, flat RTL designs, where thousands of processes wake on the same clock edge,
run up to about 2.5 times faster on 4 threads and 3.5 times on 8. Testbench-style designs and
processor models run as fast as on one thread: yavs then keeps the work on one thread. Leave it at
1 for regressions that already run tests side by side, and measure before you rely on it. Under a
token pool passed in `MAKEFLAGS`, several simulations share one CPU budget (see "Thread budget
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

When yavs runs with `--threads N` above 1 and the environment variable `MAKEFLAGS` names a
token pool (`--jobserver-auth=R,W` with two inherited pipe descriptors, or
`--jobserver-auth=fifo:PATH` with a named pipe; the jobserver protocol), the threads come from
that pool ([#801](https://github.com/ru551n/yavs/issues/801)):

* `--threads N` is the cap. Each simulation owns one implicit token, its main thread, and can take
  at most N - 1 more from the pool. The tokens decide how many of the N threads run at a time.
* A token is taken only when a group of processes is worth handing to the threads, never by
  waiting: if the pool is empty, the group runs on the main thread, as with `--threads 1`.
* Tokens taken are held while groups keep going to the threads and given back after 64 groups in a
  row that stayed on the main thread, when the run ends (normally or with an error) and on Ctrl-C,
  SIGTERM or SIGHUP. A simulation that is killed outright loses the tokens it held.
* Output, reports, waves, coverage and exit status do not depend on the tokens: they are the
  same at any token count, as at any thread count.
* Without a pool in `MAKEFLAGS`, or with `--threads 1` (the default), `MAKEFLAGS` is not read and
  `--threads` behaves as before. `--dap` runs on one thread and ignores the pool.
* A pool yavs cannot use (descriptors the parent did not pass on, so they are closed or not a
  pipe; a path that is not a named pipe; the semaphore form of Windows, which has no client yet;
  descriptors on systems other than Linux, where only `fifo:PATH` works) is error `E0128` before
  analysis (exit 2). A parent that does not share its pool with yavs removes `--jobserver-auth`
  from the child's `MAKEFLAGS`.
* Without a pool, nothing shares the CPU between simulations run side by side: do not combine a
  high `-p` with a high `--threads`, the threads of all runs add up.

A test runner that runs `-p N` simulations side by side can hand each of them `--threads` of the
core count and put the cores beyond N in the pool, adding one token each time one of its N
slots runs out of tests. The long tests at the end of a regression then use the cores that are
free. This pays only where more threads speed a simulation up: flat RTL designs (see the measured
speedup above). Processors and testbench-style designs keep their batches on one thread and gain
nothing from extra tokens. A compiled tier's compile thread (`--tier=auto`) is not counted.

## Details

* **Why mid-size batches no longer slow down.** Handing a batch to the threads has a fixed cost of
  a few microseconds and moves the processes' data between cores. A batch of a few dozen cheap
  processes costs less than that on one thread, so it stays there.
* **Why processors and testbench-style designs do not gain.** A processor's pipeline processes
  carry much state that another core's caches lack, so their batches run no faster on several
  threads, and yavs learns that and keeps them on one thread. Testbench-style designs spend most
  of their process time in processes that must stay serial (files, shared variables, protected
  objects). Their levers are single-thread speed, and a token pool gives them no extra speed.
* **What it costs when nothing goes to the threads.** About 1 to 2 percent more instructions; the
  design runs at about 0.95x to 1.0x of one thread.
* Full measurements: "Parallel kernel, step 1" and "step 2" in `docs/STATUS.md`; the design is
  `docs/design/parallel-kernel.md` in the source tree.
