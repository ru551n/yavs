Running faster
==============

This page is for making a simulation faster. The defaults are the fast settings; it says what they do and what
you can change.

.. contents:: On this page
   :local:

Tiers
-----

* ``--tier=auto`` (the default in the release archives) starts the run in the interpreter and
  compiles the functions that become hot, with the bundled Cranelift code generator. By default the simulation waits
  while a function compiles (``--tier-compile=wait``) instead of running it interpreted meanwhile. A tier-up
  thread still prepares functions alongside the run (on average about a tenth of a run's CPU time, more in a short
  test), so a run can use somewhat more than ``--threads`` cores for short periods; ``--tier=interp`` starts no such
  thread. ``--tier-compile=background`` compiles on a thread of its own
  alongside the run instead, at the lowest priority of the system: with idle cores a run that compiles a lot ends
  sooner, but each run then wants one core more while it compiles. Since a compile stops the run under ``wait``,
  in its first second of simulation work the first run of a design compiles a function only once it has spent
  about that function's compile time interpreting it; the runs after it compile what the rest of the run, as
  long as the run before, repays over a few runs. Compiles take at most about a quarter of a short run's time, the
  busiest functions for their compile time first, so the cache of a short test fills over its first few runs. A run of about half a second of simulation work or less compiles little: such a
  run would not get the compile time back. Longer runs compile the processes that keep waking up and
  what compiled code calls, and loops that run for long. A subprogram that a run spent long in within
  one call, such as a set-up loop at time 0, is compiled early in the next run, and that call goes on
  in compiled code from its loop.
* ``--tier=clift`` compiles everything before time 0. It can pay off for a long run of a
  compute-heavy design.
* ``--tier=interp`` runs the interpreter alone; it is the default of a source build without a
  compiled tier (:ref:`installation`).

The output is identical on every tier.

Warm runs
---------

yavs keeps what a run analyzed and compiled in ``yavs-cache/`` next to the work library. The first
run of a design notes it, the second stores it, and later runs of the same design, and the other
test cases of the same testbench, reuse it. Short tests then mostly skip analysis and compilation.

* Keep ``yavs-cache/`` between runs (in CI, cache the library directories: :doc:`ci`).
* An edited file, a different yavs or different options make a new entry; nothing has to be
  cleaned by hand. ``--no-cache`` turns the cache off for one run, and
  ``yavs cache clean --work=NAME:PATH`` empties it.
* The cache has a size limit per library directory, and a large suite may use more than a GB of disk.
  A suite more than about three times over the limit is slower warm than cold, by 7 to 8 percent at six
  times: split it into several libraries (:doc:`on-disk`).
* The analyzed ``std`` and ``ieee`` packages are kept once per user (:doc:`on-disk`).
* Analyze the files of one library and standard in one ``analyze`` call rather than one call per
  file: the call shares its work.
* Relative and absolute names of a file share what ``analyze`` kept: ``elab`` and ``run`` reuse it either
  way. A name that goes through ``..`` or a symlink counts as another name until it is analyzed: the units
  of that file, and those that depend on them, are analyzed once more under it. Only the time differs.

.. _huge-pages:

Huge pages
----------

A long run of a large design spends part of its time on address translation. On Linux with glibc, yavs asks the
kernel for transparent huge pages (2 MiB) for its heap and for compiled code, and keeps asking as the heap grows. On
a design with a heap of a few hundred MiB this cuts the data-TLB misses by up to half and many page faults; the effect on run time is a few percent
at most, and on a short run there is none. The memory in use grows by a few percent. Nothing is asked for when the
system does not allow it, and nothing has to be set.

* The kernel must have transparent huge pages in ``madvise`` mode, which most distributions use by default. Check
  with ``cat /sys/kernel/mm/transparent_hugepage/enabled``: ``always`` and ``madvise`` give the pages (``always`` without
  yavs asking), ``never`` gives none and yavs leaves the heap as it is. Changing that setting is the
  administrator's choice; yavs does not work around it.
* ``YAVS_HUGE_HEAP=0`` stops yavs from asking for them on the heap (:doc:`environment`); ``YAVS_CLIFT_HUGE=0`` does
  the same for compiled code.
* ``GLIBC_TUNABLES`` is never written by yavs. If you set ``glibc.malloc.top_pad``, ``trim_threshold``, ``mmap_threshold`` or ``hugetlb``
  (or the older ``MALLOC_TOP_PAD_``, ``MALLOC_TRIM_THRESHOLD_``, ``MALLOC_MMAP_THRESHOLD_``), yavs leaves the allocator to you and asks for nothing on the heap;
  ``glibc.malloc.hugetlb=1`` (glibc 2.35 and later) gives the effect for the whole heap, and is not needed otherwise. Other tunables change nothing here.
* Other systems (musl, macOS, Windows) run as before, on ordinary pages.

Antivirus on Windows
--------------------

On Windows a real-time antivirus scanner looks at every file yavs creates or changes in ``yavs-cache/``, and a run that
compiles a lot creates many (a release after 0.19.1 creates far fewer than 0.19.1 did): cold runs, and many tests run in parallel, are slowed
the most. Excluding the directory that
holds the work libraries (the output directory of your test runner), or the ``yavs.exe`` process, from real-time scanning
removes most of that cost. Exclusions are a security setting: on a managed machine, ask whoever manages it. Warm runs read
what is stored and create few files, so they gain less.

Threads
-------

``--threads=N`` helps only large, flat designs where many processes wake in the same delta cycle. Leave it at 1
(the default) unless you measured a gain, or use ``--threads=auto``, which uses more threads only while they make
the run faster: :doc:`threads`.

.. _no-asserts:

Dropping the design's asserts
-----------------------------

``--no-asserts`` drops the ``assert`` and ``report`` statements of the processes below the top-level design unit, in the
normal mode and in cycle mode alike. It is for a regression you already know the design passes: the design's own checks
cost time in every run, as much as they are many and busy, which only a run of your design shows.

* **What it trades.** The design's own checks stop running. A failure they would catch is not caught, and a design that
  ends its own test from RTL code (a ``report ... severity failure``) runs on until ``--stop-time`` or until the
  testbench ends the run.
* **What it keeps.** The processes of the top-level unit, the testbench, keep their asserts and reports, and the statements
  inside subprograms run wherever they are called from, so the check and logging calls of a verification component
  still run. Below the top-level unit is everything the testbench entity instantiates: the design, and verification
  components, bus models and checker entities.
* **When to try it.** In a job that runs the whole suite for speed after a job without it has passed, or in cycle mode
  for a process that stays in the event kernel only because of an assert it cannot evaluate (E4140,
  :doc:`cycle-mode`): such a process joins the region when its asserts are dropped.

The run starts with a note on standard error that counts the dropped statements. ``rtl_failure.vhd`` ends its test from
RTL code:

.. literalinclude:: examples/cycle/rtl_failure.vhd
   :language: vhdl
   :caption: rtl_failure.vhd

Without the option, in the normal mode and with ``--cycle``, the run ends at 275 ns with exit status 1:

.. code-block:: text

    ** Warning: 225000000fs+0: acc reached 200
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)
    ** Failure: 275000000fs+0: acc reached 250, the design ends the test
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)

With ``--no-asserts``, both statements are dropped, the note says so, and the run reaches the testbench's own end at
1022 ns with exit status 0:

.. code-block:: text

    note: `--no-asserts`: 2 assert and report statements in 1 process below the top-level unit are not evaluated

    ** Note: 1022000000fs+0: testbench finished, q=E8
        Process tb_rtl_failure.main
    ** Note: 1022000000fs+0: FINISH called with status 0

* ``--assert-summary`` and ``--exit-severity`` count only the messages that are produced.
* ``--cycle-report`` lists the dropped statements: a count per process, then each statement once, by severity, failures
  first. For ``rtl_failure.vhd`` (:ref:`cycle-report`):

  .. code-block:: text

      note: `--no-asserts`: 2 assert and report statements in 1 process below the top-level unit are not evaluated

      cycle mode: 2 of 4 processes run cycle-based, on the rising edge of tb_rtl_failure.clk
        cycle scope: the whole design (the top-level unit under its own rules)
        2 processes stay in the event kernel:
          E4111 a delay, `transport`, `reject` or a waveform (1 process):
            tb_rtl_failure.clk <= ... (rtl_failure.vhd:44)    E4111 a delayed assignment (`after`) (rtl_failure.vhd:44)
          behavioural, in the event kernel (1 process):
            tb_rtl_failure.main (rtl_failure.vhd:48)          it waits for a time
        2 asserts and reports not evaluated in cycle mode, held by 1 cycle-based process:
             2  tb_rtl_failure.dut.process (rtl_failure.vhd:16)
        each assert and report not evaluated, by severity (1 failure, 1 warning):
          failure   rtl_failure.vhd:26  tb_rtl_failure.dut.process (rtl_failure.vhd:16)
          warning   rtl_failure.vhd:24  tb_rtl_failure.dut.process (rtl_failure.vhd:16)
        cycle-based processes give no warnings of the ieee libraries (numeric_std metavalue and compare warnings)
        asserts and reports of processes of the top-level unit are evaluated: at the clock edge or after the settle in region code, else the process stays in the event kernel; those below it are dropped (--no-asserts)
        subset: 2 of 4 processes fit, 0 loop groups among them
        boundary: 1 signal leaves exported (1 nothing outside waits on), 2 inputs, 0 wake the region
        cycle-based, 2 processes:
          tb_rtl_failure.dut.process (rtl_failure.vhd:16)   clocked
          tb_rtl_failure.dut.q <= ... (rtl_failure.vhd:30)  combinational

* In VUnit the sim option is ``yavs.no_asserts`` (``True`` or ``False``, :doc:`vunit-yavs`). The option is in the
  :doc:`cli-reference` as ``--no-asserts``.

Cycle mode
----------

``--cycle`` runs the clocked and combinational RTL of a design as compiled code, evaluated once per clock edge, and
leaves the testbench in the normal simulation (:doc:`cycle-mode`).

* **What it trades.** The delta cycles inside a clock edge, ``--cov``, ``--vhpi``, activity counts and the debugger,
  and some detail in the order and delta of the design's messages. With ``--two-state`` also the metavalues
  (below).
* **When to try it.** A long run of a synchronous design whose testbench mostly drives and samples ports. It does not
  help when the testbench is most of the run, when the test is short, or when little of the design joins. It is
  experimental, so compare its results with the normal mode first (:ref:`cycle-check`).

Two-state values
----------------

``--two-state`` makes the design under test (or the scope ``--two-state=PATH`` names) compute with ``0`` and ``1``
only (:doc:`two-state`).
``--cycle --two-state`` is the fast combination: the region computes two-state code, more processes join, and it does
not follow the ``'U'`` start values. ``--two-state`` alone, in the normal mode, is a diagnostic mode and not a speed
option.

* **What it trades.** The metavalues: a register without a reset, an unconnected input or an ``'X'`` from the design
  cannot show in the scope, and the testbench reads ``0`` or ``1`` where the normal mode gives ``'U'`` or ``'X'``. A
  ``'Z'`` read in the scope is ``0``, so a bus model there that relies on ``'Z'`` can report false errors (the design
  under test leaves out models that wait for time), and pull-ups and open-drain buses in the scope break.
* **What it costs.** In the normal mode it aims to run as fast as the default; today it is slower: most unit tests
  run within a few percent of the default, some up to about half again as long, and large designs such as processor
  cores take about one and a half to four times as long. Bringing it to parity is planned.
* **When to try it.** With ``--cycle``, for speed (:ref:`cycle-check`). Alone, to check that a design does not depend on metavalues
  (with ``--init-unset`` and ``--init-x`` set to ``0``, ``1`` and ``random``, :doc:`start-values`), or to split a
  difference between cycle mode and the normal mode (:doc:`two-state`).

Finding where the time goes
---------------------------

* ``--time-report`` prints the wall and CPU time of analysis, elaboration, initialization and
  simulation when the run ends.
* ``--profile[=FILE]`` samples the run and prints the busiest processes, instances, subprograms
  and source lines (with ``FILE``, also as JSON). Ctrl-C prints the profile of the time so far.
* ``--activity[=FILE]`` counts instead of measuring: the processes that ran most often and the signals that changed most
  often (:doc:`cli`).

See also
--------

* :doc:`threads`: more threads, and when they help.
* :doc:`cycle-mode`: the opt-in mode for synchronous RTL.
* :doc:`two-state`: what the values of a cycle mode run change in a result.
* :doc:`on-disk`: where the cache lives.
* :doc:`performance`: how yavs compares with other simulators.
