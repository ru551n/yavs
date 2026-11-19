Cycle mode
==========

Cycle mode runs the clocked and combinational RTL of a design as compiled code, once per clock edge, and leaves the
testbench in the normal simulation. It is faster on long runs of synchronous designs. In return it gives up some
detail: this page says what, so that you can decide whether your tests can live with it.

.. warning::

   Cycle mode is **experimental**. Nothing changes unless you ask for it with a cycle flag. Its options
   (``--cycle``, ``--cycle-strict``, ``--cycle-report``, ``--cycle-exact-meta``, the vunit-yavs sim option
   ``yavs.cycle``), its error codes, the processes it accepts and its results may change in a later release
   without a deprecation period. Pin the yavs and vunit-yavs versions in CI.

Words used on this page and in yavs's output:

* *normal mode*: a run without a cycle flag.
* *cycle-based process*: a process that cycle mode runs as compiled code. Together they are the *region*.
* *event kernel*: where every other process runs, exactly as in the normal mode. Some messages call it the *event
  simulation*; it is the same thing.

.. contents:: On this page
   :local:
   :depth: 1

Should you use it?
------------------

yavs decides per process what joins the region. You do not mark anything in the VHDL. Cycle mode helps when:

* a design with a lot of clocked and combinational logic runs for many clock cycles, and
* the testbench mostly drives and samples ports.

It does not help, and can cost time, when:

* the testbench is most of the run: verification components, random generation, file input and output, long waits.
  Those processes stay in the event kernel and run as fast as before;
* the test is short and the region has to be compiled first (:ref:`cycle-cache`);
* little of the design joins: latches, tri-state buses, signals with several drivers, delays inside the design, real
  arithmetic. ``--cycle-report`` shows how much joins.

Some measured wall times of whole test runs, normal mode against ``--cycle``:

.. list-table::
   :header-rows: 1
   :widths: 52 16 16 16

   * - Test
     - Normal
     - ``--cycle``
     - Speedup
   * - Convolution core of a neural network accelerator
     - 6.3 s
     - 1.0 s
     - 6.3x
   * - The whole accelerator
     - 23.7 s
     - 8.8 s
     - 2.7x
   * - A small RISC-V system on chip
     - 1.26 s
     - 0.34 s
     - 3.7x
   * - Firmware tests of a system on chip
     - 4.7 s, 24.3 s
     - 1.3 s, 12.8 s
     - 3.6x, 1.9x
   * - Large CPU and system-on-chip designs where most logic stays out
     -
     -
     - 1.0x to 1.2x

The gain depends on how much of the design joins and how much of the run the testbench takes. yavs does not predict
it: run your test both ways and compare.

A run in cycle mode is a different simulation of your design, not a faster copy of the normal one. Keep your
regression running in the normal mode too.

Try it
------

On the command line
~~~~~~~~~~~~~~~~~~~

``counter.vhd`` is a counter with an asynchronous reset and two concurrent assignments. ``tb_counter.vhd`` counts
300 clock edges and checks the result. The files are in ``docs/examples/cycle`` of the source tree.

.. literalinclude:: examples/cycle/counter.vhd
   :language: vhdl
   :caption: counter.vhd

.. literalinclude:: examples/cycle/tb_counter.vhd
   :language: vhdl
   :caption: tb_counter.vhd

Analyze once, then run both ways:

.. code-block:: bash

    yavs analyze --std=2008 --work=work:build/work counter.vhd tb_counter.vhd
    yavs run --work=work:build/work tb_counter
    yavs run --cycle --work=work:build/work tb_counter

.. code-block:: text

    ** Note: 3016000000fs+0: 300 edges counted: 44
        Process tb_counter.main
    ** Note: 3016000000fs+0: FINISH called with status 0

.. code-block:: text

    cycle mode: 3 of 5 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    ** Note: 3016000000fs+0: 300 edges counted: 44
        Process tb_counter.main
    cycle mode: 9 metavalue elements read as 0 (9 at initialisation, 0 at changes of the region's inputs)
    ** Note: 3016000000fs+0: FINISH called with status 0

The testbench's output is the same. The two ``cycle mode`` lines go to standard error:

* The first says how many processes joined. When the region reads signals that nothing drives, it adds
  ``, N inputs driven by nothing read as 0`` (:ref:`cycle-two-state`). When no process joins, it says ``0 of N``
  and the run is a normal run.
* The last, printed only when it happened, says how many metavalue elements (``'U'``, ``'X'`` and so on) the region
  read as ``0``. Here they are the start values of signals without an initial value.

``yavs run`` has three cycle flags. Give one of them; two together are a usage error (exit status 2).

.. code-block:: bash

    yavs run --cycle --work=work:build/work tb_counter          # run; what does not fit stays out
    yavs run --cycle-strict --work=work:build/work tb_counter   # what does not fit is an error
    yavs run --cycle-report --work=work:build/work tb_counter   # say what would join; run nothing

``--cycle-exact-meta`` goes with one of them and keeps metavalues closer to the normal mode
(:ref:`cycle-check`).

In VUnit
~~~~~~~~

With vunit-yavs (:doc:`vunit-yavs`), the sim option ``yavs.cycle`` turns it on. It takes ``"off"`` (the default),
``"on"`` (``--cycle``) or ``"strict"`` (``--cycle-strict``). Set it for the whole run, a library, a testbench, a test
or a configuration, as any VUnit sim option. A call sets the option on the tests that exist at that moment, so make
it after adding the source files, the broad calls first and the exceptions after them.

.. code-block:: python

    vu.set_sim_option("yavs.cycle", "on")                               # every test of the run
    lib.test_bench("tb_counter").set_sim_option("yavs.cycle", "strict")  # one testbench
    lib.test_bench("tb_counter").test("my_test").set_sim_option("yavs.cycle", "off")  # one VUnit test

There is no command-line flag for it. To choose per CI job without editing the script each time, read an
environment variable of your own in ``run.py``:

.. code-block:: python

    import os
    vu.set_sim_option("yavs.cycle", os.environ.get("CYCLE", "off"))

To run every test in both modes in one run, give each testbench two configurations (a testbench that has
configurations of its own needs each of them twice):

.. code-block:: python

    for tb in lib.get_test_benches():
        tb.add_config(name="normal")
        tb.add_config(name="cycle", sim_options={"yavs.cycle": "on"})

* Another value is an error when the script sets it, with a "did you mean" hint.
* The ``cycle mode:`` lines are in each test's output. VUnit prints the output of a passing test with ``-v``.
* A test with ``yavs.cycle`` on that also asks for coverage (``--yavs-cov``, ``enable_coverage``), ``--yavs-debug``,
  ``--yavs-interactive`` or ``--yavs-surfer`` fails before yavs starts, with a message that names both options. So a
  coverage job needs ``yavs.cycle`` ``"off"``. A ``--wave`` or ``--vhpi`` in ``yavs.sim_flags`` is refused by yavs
  itself (E4102).
* To see the report of one test, set ``yavs.sim_flags`` to ``["--cycle-report"]`` for that test, with
  ``yavs.cycle`` ``"off"`` (the two flags cannot be combined). yavs prints the report and simulates nothing, so the
  test fails on purpose: use it for a look on your machine, with ``-v``, not in CI.
* ``yavs.sim_flags`` ``["--cycle-exact-meta"]`` with ``yavs.cycle`` ``"on"`` or ``"strict"`` gives a test the exact
  metavalue rules.

.. _cycle-changes:

What you give up
----------------

Read this before you trust a green run. Processes in the event kernel behave exactly as in the normal mode. For the
cycle-based processes, this is the whole list of differences:

* their ``assert`` and ``report`` statements below the top-level unit are dropped, and so are the ``ieee`` library
  warnings;
* they are two-state: ``'X'``, ``'U'`` and ``'Z'`` read as ``0`` or ``1``;
* their delta cycles are not visible, and a copied clock is the same clock as its source;
* a failed check does not print its values;
* a delayed assignment in the top-level unit stays in the event kernel, so a one-delta glitch of cycle-based logic
  feeding it can change that waveform. Compute the delayed value from registered signals, or check
  timing-sensitive waveforms in the normal mode;
* waveforms, coverage, VHPI and the debugger are refused.

If a test differs between the normal mode and ``--cycle`` for any other reason, it is a bug in cycle mode: please
`report it <https://github.com/ru551n/yavs/issues>`__.

.. _cycle-reports:

Asserts and reports below the top are dropped
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

A cycle-based process below the top-level unit produces no ``assert`` or ``report`` message, at any severity. No
cycle-based process gives the warnings of ``numeric_std`` and the other ``ieee`` and ``std`` packages (metavalues,
truncation).

So a design that ends its own test with an ``assert`` or a ``report ... severity failure`` in RTL code does not end
it in cycle mode. The run goes on until ``--stop-time`` or until the testbench ends it, and the exit status is that of
however it ends. ``rtl_failure.vhd`` is such a design:

.. literalinclude:: examples/cycle/rtl_failure.vhd
   :language: vhdl
   :caption: rtl_failure.vhd

The normal mode stops at 275 ns with exit status 1:

.. code-block:: text

    ** Warning: 225000000fs+0: acc reached 200
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)
    ** Failure: 275000000fs+0: acc reached 250, the design ends the test
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)

``--cycle`` warns at the start (W4101), then runs to the testbench's own end at 1022 ns with exit status 0:

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    warning[W4101]: 1 assert or report of severity failure is not evaluated in cycle mode
     |
     = note: rtl_failure.vhd:26: severity failure, in tb_rtl_failure.dut.process (rtl_failure.vhd:16)
     = help: these do not stop the simulation in cycle mode; run without `--cycle` to evaluate them, or with `--cycle-report` instead to list every assert and report cycle mode drops

    ** Note: 1022000000fs+0: testbench finished, q=E8
        Process tb_rtl_failure.main
    cycle mode: 8 metavalue elements read as 0 (8 at initialisation, 0 at changes of the region's inputs)
    ** Note: 1022000000fs+0: FINISH called with status 0

* W4101 comes at the start of ``--cycle`` and ``--cycle-strict`` when the dropped statements include one of severity
  ``error`` or ``failure``, or of a severity known only at run time. It gives the count and the first five, failures
  first. Dropped notes and warnings are listed only by ``--cycle-report``. ``-W error=W4101`` makes such a run end
  with exit status 1 after it has run; ``-W no-W4101`` hides the warning.
* The testbench's checks keep working. Processes of the top-level unit keep their asserts and reports: a clocked
  checker there that reads only the design's outputs joins and evaluates them at the clock edge, with the normal
  message, severity, time and effect on the exit status, if its message is built from string literals, ``&``,
  ``'image``, ``to_string``, ``to_hstring`` or ``to_ostring``. Any other top-level process with an assert stays in the
  event kernel. Within one delta cycle, the region's messages come after those of the event kernel.
* The gap is a checker or monitor *entity* instantiated below the top-level unit. It looks like design code, so it
  joins and its checks are dropped. Move such checks into a process of the top-level unit, or run the test in the
  normal mode. VUnit ``check_*`` calls in the testbench process run as before.
* ``--assert-summary`` and ``--exit-severity`` count only the messages that are produced.
* A design's own message from a function called with constant arguments (``log2(DEPTH)``, a check of a register
  map) appears once per process at 0 fs, instead of each time the statement runs, and also when the run never takes
  the branch that holds the call. A function that reports with
  severity error or failure keeps its process in the event kernel.
* A known gap: when every process around a constant joins, an ``ieee`` warning or an overflow in the constant's
  initial value (a truncating ``to_unsigned``) is not produced, and the overflow does not stop the run.

.. _cycle-two-state:

Two-state values
~~~~~~~~~~~~~~~~

A cycle-based process computes with ``0`` and ``1`` only.

* ``'U'``, ``'X'``, ``'Z'``, ``'W'`` and ``'-'`` read as ``0``; ``'H'`` reads as ``1`` and ``'L'`` as ``0``. The end of
  the run prints how many such elements were read.
* The region drives only ``'0'`` and ``'1'``: a metavalue literal such as ``(others => 'X')`` gives ``'0'``. A
  testbench that expects an ``'X'`` or a ``'Z'`` from the design sees ``'0'``, and a released ``'H'`` that the design
  copies arrives as ``'1'``. A bus model that checks read data against an expected ``'X'`` fails.
* A signal without an initial value starts at ``0``. The testbench still reads ``'U'`` at time 0, delta 0, and ``0``
  from the next delta on. Integers start at ``'left``, as in the normal mode.
* A signal the region reads that nothing drives keeps its ``'U'`` for the whole run in the normal mode, and reads as
  ``0`` in the region. A test that worked only because of that ``'U'`` (an undriven ``valid``) can now behave
  differently, or never end. The summary line counts such inputs, and ``--cycle-report`` names them. An unused field
  of a record port is counted too; it changes nothing.
* So a bug that shows as ``'X'`` or ``'U'`` in the normal mode (a register without reset, an unconnected input)
  cannot show. Run such checks in the normal mode.

``metavalue.vhd`` shows two of these: ``ticks`` has no initial value, and an ``'X'`` reaches a comparison.

.. literalinclude:: examples/cycle/metavalue.vhd
   :language: vhdl
   :caption: metavalue.vhd

The normal mode:

.. code-block:: text

    ** Note: 0fs+0: at 0 ns:  ticks=UUUU
        Process tb_metavalue.main
    ** Note: 1000000fs+0: at 1 ns:  ticks=UUUU
        Process tb_metavalue.main
    ** Warning: 15000000fs+0: NUMERIC_STD."=": metavalue detected, returning FALSE
        Process tb_metavalue.dut.process (metavalue.vhd:17)
    ** Note: 17000000fs+0: at 17 ns: eq=0 with 'X' on a
        Process tb_metavalue.main
    ** Note: 17000000fs+0: FINISH called with status 0

``--cycle``: ``ticks`` is ``0000`` from 1 ns on, the ``'X'`` on ``a`` reads as ``0`` so the comparison is true, and
the library warning is not produced.

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    ** Note: 0fs+0: at 0 ns:  ticks=UUUU
        Process tb_metavalue.main
    ** Note: 1000000fs+0: at 1 ns:  ticks=0000
        Process tb_metavalue.main
    ** Note: 17000000fs+0: at 17 ns: eq=1 with 'X' on a
        Process tb_metavalue.main
    cycle mode: 13 metavalue elements read as 0 (9 at initialisation, 4 at changes of the region's inputs)
    ** Note: 17000000fs+0: FINISH called with status 0

.. _cycle-deltas:

Delta cycles and copied clocks
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The region computes all its combinational logic at once. In the normal mode a register's output changes one delta
cycle after the edge, and logic that reads it one delta later. A testbench that samples a delta after the edge
(``wait for 0 ns``) sees the final values earlier in cycle mode. Delta numbers in messages may differ too.

.. literalinclude:: examples/cycle/delta.vhd
   :language: vhdl
   :caption: delta.vhd

The normal mode: ``r`` has not caught up with ``q`` a delta after the edge.

.. code-block:: text

    ** Note: 25000000fs+1: a delta after the edge: q=3 r=102
        Process tb_delta.main
    ** Note: 26000000fs+0: 1 ns after the edge:    q=3 r=103
        Process tb_delta.main
    ** Note: 26000000fs+0: FINISH called with status 0

``--cycle``:

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    ** Note: 25000000fs+1: a delta after the edge: q=3 r=103
        Process tb_delta.main
    ** Note: 26000000fs+0: 1 ns after the edge:    q=3 r=103
        Process tb_delta.main
    cycle mode: 8 metavalue elements read as 0 (8 at initialisation, 0 at changes of the region's inputs)
    ** Note: 26000000fs+0: FINISH called with status 0

Sample a time step after the edge (``wait for 1 ns``) or at the other edge. A testbench that reads values at a clock
edge, or a time after it, gives the same result in both modes.

.. _cycle-clock-copies:

**Copied clocks.** A clock copied without change (``clk_b <= clk;``, an aggregate of clock names, a port the clock is
connected to) is the same clock as its source in cycle mode. Registers on the copy run at the source's edge, on the
values from before that edge, as the hardware does. In the normal mode the copy changes one delta later, so a
register on the copy that reads a register of the source directly gets the source's *new* value there. A design that
relies on that delta can give other values in cycle mode. A computed clock (``not clk``, a gated clock, a divider) is
not a copy: it keeps its own edges.

.. _cycle-checks:

Check messages without values
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Integer overflow, range and index checks are still made. A failure stops the run with the normal error code, exit
status, line, time and process, but without the values. ``--cycle-exact-meta`` gives the values. A division by zero
and a quotient too large for its type give the same message. ``overflow.vhd`` counts an integer past its largest
value:

.. code-block:: text

    cycle mode: 1 of 3 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    error[E3009]: result of + cannot be represented as INTEGER (run without --cycle to see the values)
      --> overflow.vhd:11:12
       |
    11 |       n <= n + 1;  -- overflows at the third edge
       |            ^^^^^
       |
       = note: LRM 5.2.3.1, 5.2.4.1: a predefined arithmetic operation delivers the correct result
       = note: at 25 ns, delta 0, in process tb_overflow.dut.process (overflow.vhd:8)

    error: aborting due to 1 previous error

When checks on two clocks fail at the same edge, cycle mode may report the other one first.

.. _cycle-refused:

Waveforms, coverage, VHPI and the debugger are refused
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The region's signals cannot be recorded or changed from outside. ``--cycle`` and ``--cycle-strict`` stop with
E4102, before anything runs, when combined with ``--wave``, ``--surfer``, ``--cov``, ``--vhpi``, ``--activity`` or a
debug session (``--dap``, ``--interactive``, ``--do``):

.. code-block:: text

    error[E4102]: `--cycle` cannot be used with `--surfer`
     |
     = note: cycle mode keeps the signals inside its region in its own image, which this option would observe or change
     = help: run without `--cycle`

    error: aborting due to 1 previous error

Use the normal mode for those. These stay allowed: ``--profile`` (the region's time shows as the kernel's, and only
the processes outside it are listed), ``--assert-summary``, ``--shuffle``, ``--hang-report``, ``--time-report`` and
``--threads``. ``--tier`` does not apply to the region, which is always compiled. ``--cycle-report`` does not simulate,
so ``--wave`` is fine with it.

.. _cycle-check:

How to check your results
-------------------------

Before you rely on cycle mode for a suite, compare it with the normal mode once:

1. Run the suite in the normal mode and with ``--cycle`` (in VUnit, with the two configurations above).
2. Compare the pass or fail of each test, its end time and its output. The ``cycle mode:`` lines are the only
   expected extra lines.
3. For each difference, go through the list below. Anything it does not explain is a bug: please report it.

The usual causes of a difference:

* The run ends with ``N metavalue elements read as 0``: a ``'U'``, ``'X'`` or a register without reset mattered
  (:ref:`cycle-two-state`). The summary line names inputs driven by nothing.
* The run starts with W4101, or ``--cycle-report`` lists asserts not evaluated: RTL code that used to report or end
  the test does not do it now (:ref:`cycle-reports`).
* The testbench samples a delta after an edge, depends on delta numbers, or the design reads a source register on a
  copied clock (:ref:`cycle-deltas`).
* The testbench reads a signal without an initial value at time 0.

Repeat the comparison after large changes to the design or the testbench.

``--cycle-strict``
~~~~~~~~~~~~~~~~~~

Use it for a design that should join as a whole, for example in CI, so that a change that moves a process out of the
region fails the job. It keeps the *speed* you measured. It does not make results more trustworthy: it ignores
dropped asserts, and the top-level unit and behavioural processes (:ref:`cycle-selection`) are never an error. Each
process below the top that cannot join is an error, and the run stops before simulating, with exit status 2. The
errors are shown in :ref:`cycle-stays-out`.

``--cycle-exact-meta``
~~~~~~~~~~~~~~~~~~~~~~

Use it when the testbench checks for ``'X'``, ``'U'`` or ``'Z'`` from the design, or checks reset state, and to see the
values of a failed check. It is not the normal mode: it is closer.

* A process that assigns a metavalue literal (``(others => 'X')``) to a signal the testbench reads, or passes on a
  ``'Z'`` from outside, stays in the event kernel (E4137), so the testbench sees the normal mode's value.
* Registers and signals without an initial value show the testbench ``'U'`` while their value depends on that start
  value, for up to 1024 clock edges. To know which bits depend on it, yavs runs the region a second time with those
  start values at ``1`` and compares the two. A bit fed only by ``'X'``, ``'Z'``, ``'W'`` or ``'-'`` shows ``'X'``.
  The event from ``'U'`` to ``0`` at time 0 is gone, and a bit that does not depend on the start value is ``0`` or
  ``1`` (``q and not q`` gives ``0`` where the normal mode gives ``'U'``).
* Failed checks give their values, with the normal mode's message.
* The price is speed: more processes stay out, and the region does more work at first.

Known differences from the normal mode, also with this flag:

* An ``'X'`` or ``'U'`` that comes in from outside the region arrives at the testbench as ``'0'`` once the start values
  are no longer followed.
* A choice (``if``, ``case``) on a ``'U'`` can take the branch for ``0``. A choice on a variable can show ``'U'`` where
  the normal mode has a value.
* A clocked process can keep a register's old value where only the run on start values of ``1`` changed it.
* ``numeric_std`` arithmetic on a metavalue gives ``'U'`` or ``'X'`` only in the bits that depend on it, not all
  ``'X'``.
* A ``'U'`` that comes back to the region through a process outside reads as ``0``.

With ``metavalue.vhd``, ``ticks`` now stays ``'U'`` as in the normal mode. ``eq`` is still ``1`` (the register kept its
value, as in the third point), where the normal mode gives ``0``:

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    ** Note: 0fs+0: at 0 ns:  ticks=UUUU
        Process tb_metavalue.main
    ** Note: 1000000fs+0: at 1 ns:  ticks=UUUU
        Process tb_metavalue.main
    ** Note: 17000000fs+0: at 17 ns: eq=1 with 'X' on a
        Process tb_metavalue.main
    cycle mode: 13 metavalue elements read as 0 (9 at initialisation, 4 at changes of the region's inputs)
    cycle mode: 'U' start values followed on a second image: still followed when the run ended, after 2 clock edges
    ** Note: 17000000fs+0: FINISH called with status 0

The extra summary line says how following the start values ended. The "second image" is that second run:

* ``the images agreed after N clock edges``: nothing depends on a start value any more. yavs compares every 16
  edges, so N is at least 16;
* ``stopped at the cap of N clock edges``;
* ``stopped after N clock edges: a check failed on it in ...``: a range or index check failed on the start values of
  ``1`` only. It never stops the run;
* ``still followed when the run ended, after N clock edges``.

.. _cycle-report:

Reading ``--cycle-report``
--------------------------

``--cycle-report`` checks every process, prints the report on standard error and ends with exit status 0. It
compiles and simulates nothing, so it costs little. ``--cycle-report=FILE`` writes the report to FILE and prints
nothing; a FILE it cannot write is error E4103, before any work.

``left_out.vhd`` is a small design with four things cycle mode does not take, and a testbench:

.. literalinclude:: examples/cycle/left_out.vhd
   :language: vhdl
   :caption: left_out.vhd
   :linenos:

.. code-block:: bash

    yavs run --cycle-report --work=work:build/work tb_left_out

.. code-block:: text

    cycle mode: 1 of 7 processes run cycle-based, on the rising edge of tb_left_out.clk
      6 processes stay in the event kernel:
        E4111 a delay, `transport`, `reject` or a waveform (2 processes):
          tb_left_out.clk <= ... (left_out.vhd:62)    E4111 a delayed assignment (`after`) (left_out.vhd:62)
          tb_left_out.dut.p_slow (left_out.vhd:37)    E4111 a delayed assignment (`after`) (left_out.vhd:40)
        E4119 a latch (1 process):
          tb_left_out.dut.p_latch (left_out.vhd:29)   E4119 a latch: tb_left_out.held(0) is not assigned on every path (this statement leaves it unassigned) (left_out.vhd:31)
        E4121 an incomplete sensitivity list (1 process):
          tb_left_out.dut.p_sum (left_out.vhd:45)     E4121 an incomplete sensitivity list: the process reads tb_left_out.dut.b(0), which the list does not name (left_out.vhd:47)
        E4127 its history is read outside the region (1 process):
          tb_left_out.dut.q <= ... (left_out.vhd:26)  E4127 history of a signal read outside the region: `'last_event` of tb_left_out.q, read by tb_left_out.main (left_out.vhd:77)
        behavioural, in the event kernel (1 process):
          tb_left_out.main (left_out.vhd:67)          it waits for a time
      cycle-based processes give no warnings of the ieee libraries (numeric_std metavalue and compare warnings)
      asserts and reports of processes of the top-level unit are evaluated: at the clock edge in region code, else the process stays in the event kernel
      subset: 1 of 7 processes fit, 0 loop groups among them
      boundary: 1 signal leaves exported (0 nothing outside waits on), 1 inputs, 0 wake the region
      cycle-based, 1 process:
        tb_left_out.dut.p_reg (left_out.vhd:20)     clocked

Read it from the top:

* The first line: how many processes join, and on which clock edges they run.
* The processes that stay out, grouped by reason, the most common first. Each row has the process, where it starts,
  and the reason at the line that causes it. A concurrent assignment shows as ``name <= ...``. The testbench's clock
  (``clk <= not clk after ...``) is on almost every list and is expected. *Behavioural* processes are testbench
  code by nature (:ref:`cycle-selection`).
* Two fixed lines: no ``ieee`` library warnings from the region, and the top-level unit's asserts are evaluated.
* ``subset``: how many processes fit the rules on their own. A few may still stay out because of the design around
  them. A *loop group* is a set of processes that read each other's outputs without forming a combinational loop;
  they cost more per clock.
* ``boundary``: the signals between the region and the event kernel. ``exported`` are the signals the region drives
  for processes outside it, ``inputs`` those it reads from them, and ``wake the region`` the inputs that make it run
  between clock edges. Every such signal is converted and compared, so large numbers here cost speed. A ``wakes:``
  line may follow.
* The cycle-based processes, each ``clocked`` or ``combinational``, and the inputs nothing drives, if any.

When cycle-based processes hold ``assert`` or ``report`` statements, the report lists them after the processes that
stay out: a count per process, then each statement once, by severity. For ``rtl_failure.vhd``:

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, on the rising edge of tb_rtl_failure.clk
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
      asserts and reports of processes of the top-level unit are evaluated: at the clock edge in region code, else the process stays in the event kernel
      subset: 2 of 4 processes fit, 0 loop groups among them
      boundary: 1 signal leaves exported (1 nothing outside waits on), 2 inputs, 0 wake the region
      cycle-based, 2 processes:
        tb_rtl_failure.dut.process (rtl_failure.vhd:16)   clocked
        tb_rtl_failure.dut.q <= ... (rtl_failure.vhd:30)  combinational

.. _cycle-stays-out:

Why a process stays out, and how to fix it
------------------------------------------

A process that stays out is not an error under ``--cycle``: it runs in the event kernel, which is always correct.
You only need to change something if you want it to join, for speed, or under ``--cycle-strict``.

.. _cycle-selection:

What joins
~~~~~~~~~~

A process joins when yavs can show that running it once per clock edge gives the normal mode's result, apart from the
differences above.

* **Clocked processes:** ``if rising_edge(clk) then ... end if;`` (also ``falling_edge``, or ``clk'event and clk =
  '1'``, optionally ``and`` more conditions) with the clock in the sensitivity list; the same with an asynchronous
  reset branch first (``if rst = '1' then ... elsif rising_edge(clk) then``, clock and reset in the list); a process
  whose only wait is a first ``wait until rising_edge(clk);``; ``q <= d when rising_edge(clk);``. Several clocks and
  both edges are fine. The clock must come from outside the region, usually the testbench.
* **Combinational processes:** a complete sensitivity list or ``all``, every signal and variable assigned on every
  path, no combinational loop. Concurrent assignments count as combinational processes.
* **What they may contain:** logic and bit types, booleans, integers, enumerations, records, arrays and memories;
  ``numeric_std`` and ``std_logic_1164`` operators; ``if``, ``case`` and ``for`` loops with bounds known at
  elaboration; generics and ``generate``; calls of functions and procedures that have a VHDL body. Each element of a
  signal needs one driver.
* **Behavioural processes** stay in the event kernel and are never an error: a wait for a time, ``wait;``, several
  waits, a call of a procedure that waits, files or text I/O, or a declared protected object, file or access value.
  This is testbench code, also when it is a bus model below the top.
* **Every process of the top-level unit** is never an error under ``--cycle-strict``.

The codes
~~~~~~~~~

The report and ``--cycle-strict`` give each reason a code. ``yavs --explain E4119`` prints the full text of one;
:doc:`error-codes` lists them all. *Limitation* marks a gap in yavs, not a problem in the design.

.. list-table::
   :header-rows: 1
   :widths: 10 40 50

   * - Code
     - The process
     - How to make it join
   * - E4101
     - Uses a construct yavs does not implement yet (the message names it), such as a comparison with a metavalue
       literal (``= 'U'``) or a ``while`` loop
     - Limitation. Write it another way.
   * - E4110
     - Has a wait other than a first ``wait until <edge>;``
     - Write a clocked process, or a combinational one with a sensitivity list.
   * - E4111
     - Uses ``after``, ``transport``, ``reject`` or a waveform of several elements
     - Assign without a delay in the design. Keep delays in testbench models.
   * - E4112
     - Reads signal history (``'last_value``, ``'stable``, ...) or an edge of another signal
     - Use an edge test of its own clock, or keep the old value in a register.
   * - E4113
     - Computes a ``real`` or ``time`` value at run time
     - Use an integer or a constant.
   * - E4114
     - Uses a shared variable with other processes, a protected method, an allocator or an access value
     - Pass values through signals.
   * - E4115
     - Is postponed, or uses guarded blocks, guarded signals or ``null`` waveforms
     - Use a plain process and a signal with one driver.
   * - E4116
     - Uses ``force`` or ``release``
     - Force from the testbench.
   * - E4117
     - Calls a subprogram without a VHDL body, or (combinational) one that changes what it is not given
     - Give it a body, or pass what it changes as parameters.
   * - E4118
     - Uses ``'Z'``
     - Keep tri-state buses in the testbench, or use a separate enable.
   * - E4119
     - Is a latch: a signal not assigned on every path
     - Give it a default value at the start of the process.
   * - E4120
     - Reads a variable before assigning it (combinational)
     - Assign it at the start of the process.
   * - E4121
     - Has an incomplete sensitivity list
     - Use ``process (all)``, or list every signal it reads.
   * - E4122
     - Is part of a combinational loop
     - Break the loop with a register.
   * - E4123
     - Is clocked in another shape
     - Use one of the clocked forms above, with nothing outside the edge test.
   * - E4124
     - Shares a signal element with another driver, or a resolution function of the design
     - Drive each element from one process.
   * - E4125
     - Uses a clock made inside the design (gated, divided)
     - Make the clock in the testbench, or use a clock enable.
   * - E4126
     - Uses a construct yavs cannot follow far enough
     - Limitation. Write it more simply.
   * - E4127
     - Drives a signal whose history a process outside reads
     - Let the reader keep its own copy of the old value, or use an edge test.
   * - E4128
     - Drives a signal a ``force`` or ``release`` of the design can change (a force through a signal parameter, or
       any force under ``--cycle-exact-meta``, keeps every process out)
     - Limitation. Force from the testbench instead.
   * - E4129
     - Feeds a process outside the region that depends on delta cycles (the message names it). A delayed
       assignment (``after``, ``transport``, ``reject``) below the top-level unit is such a process: a one-delta
       glitch in the normal mode removes or adds its pending value, so it stays out, and so does the logic
       feeding it
     - Let that reader join too (complete sensitivity list, no latch), or make it clocked. For a delayed
       assignment, compute the delayed value from registered signals, or move the delay to the testbench.
   * - E4130
     - Is logic between clocks whose edges come in different delta cycles
     - Clock both from one clock, or make the second a plain copy (``clk_b <= clk_a;``).
   * - E4131
     - Computes a clock from signals the region drives
     - Make the clock in the testbench, or use a clock enable.
   * - E4132
     - Is a register reset by another register that drives a clock or reset
     - Use a synchronous reset, or one that does not come from a register.
   * - E4133
     - Uses a memory written in its reset branch, at fixed and run-time indices both, or read outside the region
     - Limitation. Write it at run-time indices at the clock edge, and read it inside the design.
   * - E4134
     - Drives part of a signal whose other part a process outside drives
     - Limitation. Drive all of it from logic that joins, or split the signal.
   * - E4135
     - Drives a signal that also gets a value from a port connection or a constant
     - Limitation. Drive it from one process only.
   * - E4136
     - Is a top-level process whose assert reads a signal the region does not drive alone (report only)
     - Nothing to fix: it keeps the testbench's check exact.
   * - E4137
     - With ``--cycle-exact-meta``: would hide a metavalue from the testbench
     - Assign ``'0'`` or ``'1'``, or run without ``--cycle-exact-meta``.
   * - E4138
     - Uses an external name
     - Limitation. Pass the signal through a port.
   * - E4139
     - Has a loop with unknown bounds, a very large loop, or ``exit`` or ``next`` of an outer loop
     - Use bounds from constants or generics; jump only out of the innermost loop.
   * - E9999
     - Hit an internal error
     - A bug in yavs: please report it. ``--cycle`` keeps the process in the event kernel.

The errors of ``--cycle-strict``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Each error names the construct, marks the statement (and the other process, for a cause that spans two), lists the
processes it keeps out, and says why and how to fix it. There is one error per code and statement, sorted by file and
line. At most 40 are printed; a last line counts the rest, and ``--cycle-report`` lists them all. ``left_out.vhd``:

.. code-block:: bash

    yavs run --cycle-strict --work=work:build/work tb_left_out

.. code-block:: text

    error[E4119]: a latch is not supported in cycle mode
      --> left_out.vhd:31:5
       |
    31 | /     if en = '1' then
    32 | |       held <= d;
    33 | |     end if;
       | |___________^ tb_left_out.held(0) is not assigned on every path (this statement leaves it unassigned)
       |
       = note: in process tb_left_out.dut.p_latch (left_out.vhd:29)
       = note: why: a signal that is not assigned on every path keeps its old value, which is state, not combinational logic
       = help: assign it on every path, for example with a default assignment at the start of the process; or run with `--cycle` to leave this process in the event simulation

    error[E4111]: a delayed assignment (`after`) is not supported in cycle mode
      --> left_out.vhd:40:7
       |
    40 |       slow <= d after 1 ns;
       |       ^^^^^^^^^^^^^^^^^^^^^
       |
       = note: in process tb_left_out.dut.p_slow (left_out.vhd:37)
       = note: why: cycle-based code has no time inside a clock cycle, so it cannot keep a delay
       = help: assign without a delay (`q <= d;`) in the design; keep delays in testbench models; or run with `--cycle` to leave this process in the event simulation

    error[E4121]: an incomplete sensitivity list is not supported in cycle mode
      --> left_out.vhd:47:16
       |
    47 |     sum <= d + b;
       |                ^ the process reads tb_left_out.dut.b(0), which the list does not name
       |
       = note: in process tb_left_out.dut.p_sum (left_out.vhd:45)
       = note: why: the event simulation does not run the process when that signal changes, so its result is not a function of what it reads, and cycle-based code would compute another result
       = help: use `process (all)`, or name every signal the process reads in its sensitivity list; or run with `--cycle` to leave this process in the event simulation

    error[E4127]: history of a signal read outside the region is not supported in cycle mode
      --> left_out.vhd:77:43
       |
    26 |   q <= reg;
       |   --------- tb_left_out.dut.q <= ... (left_out.vhd:26) drives tb_left_out.q
    ...
    77 |     report "q last changed " & time'image(q'last_event) & " ago";
       |                                           ^^^^^^^^^^^^ `'last_event` of tb_left_out.q, read by tb_left_out.main
       |
       = note: in process tb_left_out.dut.q <= ... (left_out.vhd:26)
       = note: why: a process outside the region reads the history of a signal this process drives, and cycle-based code keeps no history of the signals it drives
       = help: let the reader keep its own copy of the old value, or use an edge test instead of `'last_value` or `'last_event`; or run with `--cycle` to leave this process in the event simulation

    error: aborting due to 4 previous errors

E4127 is the one to watch for: a testbench that reads ``'last_event``, ``'last_value``, ``'stable`` and the like of a
design signal keeps the processes that drive it out, silently under ``--cycle``, and as an error under
``--cycle-strict`` even though the reader is the testbench. Edge tests such as ``rising_edge`` are fine.

When it is slower
-----------------

Read the first line of ``--cycle-report``:

* **Few processes joined.** The rows say what keeps the rest out; start with the largest group and the table above.
  Large numbers in the ``boundary`` line mean the split between region and event kernel is expensive. A clocked
  checker of the top-level unit with an assert on a signal the region does not drive alone stays out by design.
* **The testbench is most of the run.** ``--profile`` shows where the time goes (:doc:`running-faster`). Cycle mode
  does not speed up that part.
* **The test is short.** The first run compiles the region, which takes longer the larger the region is. Later runs
  take it from the cache (:ref:`cycle-cache`).
* **An idle design is cheap.** When a whole clock edge would change nothing in the cycle-based part, that edge is
  skipped. An idle block inside a busy design still costs its full share.

.. _cycle-mode-ci:

Using it in CI
--------------

* Keep one job in the normal mode as the sign-off. Run cycle mode as a second, faster job, or for the long tests
  that you compared once (:ref:`cycle-check`).
* Keep coverage, waveform and debug jobs in the normal mode: with ``yavs.cycle`` on, they fail every test.
* Use ``--cycle-strict`` (``"strict"`` in VUnit) on designs that should join as a whole, so that a change that moves
  a process out fails the job.
* Grep the test output for the ``cycle mode:`` lines and ``warning[W4101]``: a new W4101, or a metavalue count that
  changes, is worth a look. To fail the job on it instead, pass ``-W error=W4101`` (in VUnit, in ``yavs.sim_flags``).
* Keep the library directories between runs, so the cache is warm (:doc:`ci`). On a cold cache, tests that start at
  once each compile the region. That is safe, only slower.
* Pin the yavs and vunit-yavs versions: cycle mode may change between releases.

.. _cycle-cache:

The cache
---------

The first run of a design checks the processes and compiles the region. Later runs of the same design take both from
the compilation cache of the work library, in ``yavs-cache/cycle`` inside the library directory (:doc:`on-disk`).
A warm run starts about as fast as a run without ``--cycle``. The test cases of one testbench share the cache,
unless the design reads generics that differ between them when it is elaborated (a width, a depth).

* The cycle part of the cache keeps at most 2 GiB per library directory. ``YAVS_CYCLE_CACHE_MIB`` changes the limit, in
  MiB (a whole number of at least 1; anything else is E4104). At the limit nothing new is stored: the designs past it
  compile again on every run. There is no message; it is only slower.
* ``yavs cache clean --work=NAME:PATH`` empties the library's cache, cycle part included. It also empties the shared
  ``std`` and ``ieee`` store (``YAVS_STD_CACHE``) unless you add ``--std-cache=off``.
* ``--no-cache`` neither reads nor writes it: such a run compiles every time.
* An edit, other options or another yavs version make new entries. Nothing has to be cleaned by hand.
* A design whose elaboration writes files, or reads environment variables, the clock or files that are not text,
  checks its processes again on every run; its compiled functions are still reused.
* The region compiles on as many threads as ``--threads`` gives; under vunit-yavs, on the CPUs no other test uses
  (:doc:`threads`). The result is the same on any number of threads.

Troubleshooting
---------------

**A test passes in the normal mode and fails with** ``--cycle``. Go through the causes in :ref:`cycle-check`. A bus
model that expects ``'X'`` from the design fails by design: run that test in the normal mode.

**A test hangs, or ends at another time.** If the summary line counts inputs driven by nothing, ``--cycle-report``
names them: drive them in the testbench. If the RTL ended the test with an assert, the run now goes on: give it a
``--stop-time``, or move the check into the testbench. If the testbench waits a delta after an edge, see
:ref:`cycle-deltas`.

**A test fails in the normal mode and passes with** ``--cycle``. An assert below the top-level unit was dropped
(W4101), or a metavalue was read as ``0``. Trust the normal mode.

**Nothing joins.** The summary says ``0 of N``. Run ``--cycle-report`` and look at the largest group. A design
whose RTL uses ``after`` delays or ``'Z'`` everywhere does not fit.

**An option is refused with E4102.** Waves, coverage, VHPI and the debugger need the normal mode
(:ref:`cycle-refused`).

**The first run is slow.** It compiles the region; see :ref:`cycle-cache`.

**Where do I report a difference?** On the `issue tracker <https://github.com/ru551n/yavs/issues>`__, with the
design or the smallest part that shows it, the command line, and the output of ``--cycle-report``. Report internal
errors (E9999) too.

Reference
---------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Flag of ``yavs run``
     - What it does
   * - ``--cycle``
     - Runs what fits cycle-based and the rest in the event kernel. Selection never fails the run.
   * - ``--cycle-strict``
     - As ``--cycle``, but a process below the top-level unit that is not behavioural and cannot join is an
       error; the run stops before simulating, exit status 2.
   * - ``--cycle-report[=FILE]``
     - Prints the report (or writes it to FILE, printing nothing), compiles and simulates nothing, exit status 0.
       An unwritable FILE is E4103.
   * - ``--cycle-exact-meta``
     - With one of the three above: exact metavalues for the testbench and values in failed checks. Alone it is a
       usage error, exit status 2.

* The three mode flags exclude each other (exit status 2). ``yavs analyze`` and ``yavs elab`` take none of them.
* vunit-yavs: the sim option ``yavs.cycle`` (``"off"``, ``"on"``, ``"strict"``), :doc:`vunit-yavs`.
* Environment: ``YAVS_CYCLE_CACHE_MIB``, the limit of the cycle cache in MiB (default 2048), :doc:`environment`.
  The other ``YAVS_CYCLE_*`` variables are for developers of yavs.
* Codes: W4101, E4101 to E4104 and E4110 to E4139 in :doc:`error-codes`, and ``yavs --explain CODE``.

See also
--------

* :doc:`cli-reference`: the cycle flags.
* :doc:`vunit-yavs`: the ``yavs.cycle`` sim option.
* :doc:`running-faster`: the other ways to a faster run.
* :doc:`threads`: threads and the token server, which the region's compilation also uses.
