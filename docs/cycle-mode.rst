Cycle mode
==========

Cycle mode runs the synchronous RTL of a design as compiled code that is evaluated once per clock edge, and leaves
the testbench in the ordinary event simulation. You turn it on per run with ``--cycle``; without it nothing changes.
It trades some of the simulator's detail for speed, and this page lists what it trades, so that you can decide
whether your tests can live with it.

.. note::

   Cycle mode is new and opt-in. It is developed on a side branch of the source repository and is
   not in any release yet, so you need a build from that branch. The set of designs it accepts is still growing: a
   process it cannot take yet simply stays in the event simulation (see :ref:`cycle-report`), and the list in this
   page is the behaviour of the branch at the time of writing.

.. contents:: On this page
   :local:

What it is and when to use it
-----------------------------

Normally yavs simulates every process in delta cycles, with nine-valued logic, the history of every signal and the
reports and assertions of every process. Cycle mode takes the processes that are plain synchronous or combinational
RTL, decides their order once when the design is elaborated, compiles them together with Cranelift and runs them
once per clock edge on two-state values (``0`` and ``1``). Together they are the *region*. Every other process,
usually the testbench, stays in the event simulation and talks to the region through the signals it reads and drives.
Large combinational processes whose inputs did not change since their last run are not run again, and between clock
edges only the logic that a changed input reaches runs, so the part of a design that is idle in a clock costs little.

Which processes join is decided by yavs, per process (:ref:`cycle-selection`). You do not mark anything in the
VHDL.

Cycle mode helps when:

* a design with a lot of clocked and combinational logic is simulated for many clock cycles, and
* the testbench around it mostly drives and samples ports.

It does not help, and can cost time, when:

* the run is dominated by the testbench: verification components, random generation, file or text input and
  output, long waits. These processes stay in the event simulation and run at the same speed as before.
* the test is very short and the compiled code has to be made first (:ref:`cycle-cache`).
* most of the logic does not join the region: latches, tri-state buses, signals with several drivers, delays
  inside the design, real arithmetic. ``--cycle-report`` tells you how much did.

Cycle mode always runs cycle-based what it can; yavs does not estimate beforehand whether that pays. Whether it is
faster for your design is for you to measure: run it with and without ``--cycle``.

It also changes what a run can show, as :ref:`cycle-changes` lists. A run in cycle mode is a different simulation
of your design, not a faster copy of the normal one. Run your regression in the normal mode too.

Turn it on
----------

There are three flags of ``yavs run``. They are alternatives; giving two is a usage error (exit status 2).

.. code-block:: bash

    yavs run --cycle --work=work:build/work tb_counter            # leave out what does not fit
    yavs run --cycle-strict --work=work:build/work tb_counter     # what does not fit is an error
    yavs run --cycle-report --work=work:build/work tb_counter     # say what would join, run nothing

* ``--cycle`` selects the processes that can join, prints one line on standard error that says how many joined,
  and runs the simulation. A second line at the end says how many metavalues were read as ``0``
  (:ref:`cycle-changes`), if there were any. Selection never causes an error.
* ``--cycle-strict`` is ``--cycle`` for designs that should join completely: a process below the top-level
  design unit that looks like RTL and cannot join is an error (:ref:`cycle-strict`).
* ``--cycle-report`` analyses the design, prints the full report of which processes join and why the others do not,
  and ends with exit status 0 before anything is compiled or simulated (:ref:`cycle-report`).
  ``--cycle-report=FILE`` writes the report to a file instead of standard error.

``yavs analyze`` takes none of them, and ``yavs elab`` does not take them. The region is compiled whatever
``--tier`` says, and ``--threads`` and ``--shuffle`` are accepted. The options ``--wave``, ``--cov``, ``--vhpi`` and
a debug session are refused (:ref:`cycle-changes`).

From a VUnit run script the sim option ``yavs.cycle`` does the same, per library, testbench, test or configuration:

.. code-block:: python

    # Every test of the library runs cycle-based.
    lib.set_sim_option("yavs.cycle", "on")
    # One test case on its own is strict.
    lib.test_bench("tb_counter").test("holds_without_enable").set_sim_option("yavs.cycle", "strict")

The values are ``"off"`` (the default), ``"on"`` (``--cycle``) and ``"strict"`` (``--cycle-strict``); anything else
is an error at the time of the call. The line ``cycle mode: N of M processes run cycle-based`` is part of the test's
output, shown with ``-v`` for a test that passes. A test that also asks for coverage, a debug session,
``--yavs-interactive`` or ``--yavs-surfer`` fails before yavs starts, with a message that names both options. To see
the report of one test, give it ``yavs.sim_flags`` ``["--cycle-report"]``: yavs prints the report and does not
simulate, so that test fails on purpose. :doc:`vunit-yavs` has the option in its table.

A first run
-----------

``counter.vhd`` is a clocked counter with an asynchronous reset and two concurrent assignments. ``tb_counter.vhd``
is a testbench that counts 300 clock edges and checks the result.

.. literalinclude:: examples/cycle/counter.vhd
   :language: vhdl
   :caption: counter.vhd

.. literalinclude:: examples/cycle/tb_counter.vhd
   :language: vhdl
   :caption: tb_counter.vhd

The files are in ``docs/examples/cycle`` of the source tree. Analyze them once, then run the design both ways:

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

The testbench's own output is the same. The two ``cycle mode`` lines are on standard error. The second one
is the first sign of what changes: nine elements of signals that start as ``'U'`` were read as ``0``.

.. _cycle-selection:

What runs cycle-based
---------------------

Selection is automatic and per process. A process joins the region when yavs can prove that it is clocked or
combinational logic that a static schedule evaluates to the same result as the event simulation, apart from the
differences in :ref:`cycle-changes`. If it cannot prove that, the process stays in the event simulation, which is
always correct, and the report says why.

**Clocked processes** join in these forms:

* ``if rising_edge(clk) then ... end if;``, also ``falling_edge(clk)`` and ``clk'event and clk = '1'``, optionally
  followed by ``and`` and more conditions, with a sensitivity list of the clock;
* the same with an asynchronous reset in front: ``if rst = '1' then ... elsif rising_edge(clk) then ... end if;``,
  where the sensitivity list names the clock and the reset signals;
* a process that starts with ``wait until <edge>;`` and has no other wait;
* the concurrent form ``q <= d when rising_edge(clk);``.

Several clocks and both edges of a clock are fine. A clock must come from outside the region, for example from the
testbench: a clock made by a register or by combinational logic inside the design, and the logic that uses it, stays
in the event simulation (E4125).

**Combinational processes** join when they are a function of what they read: a sensitivity list that names every
signal they read (or ``all``), every signal and variable they assign assigned on every path (no latch, no state),
and no combinational loop. Concurrent simple, conditional and selected assignments join in the same way.

**What they may contain.** Logic and bit types, booleans, integers, enumerations, records, arrays and memories
(arrays of more than a few thousand elements, written at run-time indices, or held in a shared variable that one
clocked process alone uses), ``numeric_std`` and
``std_logic_1164`` operators, ``case``, ``if``, ``for`` loops whose bounds are known when the design is elaborated
(constants and generics), slices and indices bounded by variables whose values are known on every path
(``lo := hi + 1; v(hi downto lo) := ...``), conditions on calls of the design's pure functions of generics, generics, ``generate`` statements, and calls of functions and procedures that have a
body. A signal needs one source: one process drives each element of it.

**What stays in the event simulation**, and the code the report gives it (every code has its own entry in
:doc:`error-codes`, under "Cycle mode"):

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - The process
     - Reason
   * - Waits for a time, ``wait;``, has several waits, calls a procedure that waits, uses files or text
       input and output, access values or protected objects
     - *Behavioural*: the testbench kind of process. Not an error in any mode.
   * - Is in the top-level unit and holds an ``assert`` or ``report`` that cycle mode cannot evaluate in the
       region: outside the clock edge of a clocked process (a combinational process, a concurrent assert), with a
       message of another form, or reading a signal the testbench drives
     - Behavioural, so that the checks of a testbench run (:ref:`cycle-changes`).
   * - Uses ``after``, ``transport`` or ``reject``
     - E4111: no time inside a clock cycle. This includes the usual ``clk <= not clk after 5 ns;``, which is
       the testbench's job and stays where it is.
   * - Reads signal history of a signal that is not its own clock (``'last_value``, ``'stable``, ``'delayed``, ...),
       or its history is read by a process outside the region
     - E4112, E4127.
   * - Computes a ``real`` or ``time`` value at run time
     - E4113.
   * - Uses ``'Z'``, a guarded signal, ``force`` or ``release``, a postponed process
     - E4118, E4115, E4116.
   * - Is a latch, reads a variable before assigning it, has an incomplete sensitivity list, is part of a
       combinational loop
     - E4119, E4120, E4121, E4122.
   * - Is combinational logic that feeds, back to the registers (processes that act at clock edges alone), a
       process outside the region whose result may depend on the delta cycles its inputs change in: one that
       keeps a value of an earlier run (a combinational loop included, such as ``l <= d when en = '1' else
       l;``), or, below the top-level unit, a process that drives a signal and has an incomplete sensitivity
       list or that cycle mode could not show keeps no value (a process with a ``'Z'`` and a variable, for
       example). A concurrent assignment with a final ``else`` that reads nothing it drives and stays out for
       its ``'Z'`` or its ``after`` alone does not count, nor does a process that acts at clock edges alone.
       Testbench code (the top-level unit, and a monitor that drives no signal) sees the settled values
       earlier, as described below
     - E4101. The region settles such logic in one delta cycle where the normal mode takes several, so that
       process would see other values or run another number of times.
   * - Shares a signal element with another process, or the signal has a resolution function of its own
     - E4124.
   * - Is clocked in a form that is not one of those above, or uses a clock made inside the region
     - E4123, E4125.
   * - Compares with a metavalue literal (``= 'U'``), has a ``while`` loop or a ``for`` loop whose bounds are
       not known when the design is elaborated, or another construct cycle mode cannot take *yet*
     - E4101: in the subset, not implemented yet. This is the one reason that is expected to shrink.

.. _cycle-report:

Reading the report
~~~~~~~~~~~~~~~~~~

``--cycle-report`` shows the decision for every process. This is the report for ``left_out.vhd`` of the examples,
a small design with a testbench and four things cycle mode does not take:

.. literalinclude:: examples/cycle/left_out.vhd
   :language: vhdl
   :caption: left_out.vhd
   :lines: 1-49

.. code-block:: bash

    yavs run --cycle-report --work=work:build/work tb_left_out

.. code-block:: text

    cycle mode: 1 of 7 processes run cycle-based, on the rising edge of tb_left_out.clk
      6 processes stay in the event kernel:
        E4111 a delay, `transport`, `reject` or a waveform (2 processes):
          tb_left_out.clk <= ... (left_out.vhd:62)    E4111 a delayed assignment (`after`) needs time inside a cycle (left_out.vhd:62)
          tb_left_out.dut.p_slow (left_out.vhd:37)    E4111 a delayed assignment (`after`) needs time inside a cycle (left_out.vhd:40)
        E4119 a latch (1 process):
          tb_left_out.dut.p_latch (left_out.vhd:29)   E4119 it does not assign tb_left_out.held(0) on every path, so it keeps its value: a latch (this statement leaves it unassigned on a path) (left_out.vhd:31)
        E4121 an incomplete sensitivity list (1 process):
          tb_left_out.dut.p_sum (left_out.vhd:45)     E4121 it reads tb_left_out.dut.b(0), which its sensitivity list does not name: the normal mode would not run it when that changes (left_out.vhd:47)
        E4127 its history is read outside the region (1 process):
          tb_left_out.dut.q <= ... (left_out.vhd:26)  E4127 tb_left_out.main reads `'last_event` of tb_left_out.q, and the region keeps no history of the signals it drives: its drivers stay in the event kernel (left_out.vhd:77)
        behavioural, in the event kernel (1 process):
          tb_left_out.main (left_out.vhd:67)          it waits for a time
      cycle-based processes give no warnings of the ieee libraries (numeric_std metavalue and compare warnings)
      asserts and reports of processes of the top-level unit are evaluated: at the clock edge in region code, else the process stays in the event kernel
      subset: 1 of 7 processes fit, 0 loop groups among them
      boundary: 1 signal leaves exported (0 nothing outside waits on), 1 inputs, 0 wake the region
      cycle-based, 1 process:
        tb_left_out.dut.p_reg (left_out.vhd:20)     clocked

* The first line says how many processes join and on which clock edges they run.
* Then come the processes that stay, grouped by reason, the most common reason first. A row has the process (an
  instance path and where it starts) and the reason with the position of the construct that causes it. A
  concurrent assignment is shown as ``name <= ...``. A delayed clock like ``clk <= not clk after ...`` is on
  almost every list and is expected. *Behavioural* processes are the ones that are testbench by nature.
* If cycle-based processes hold ``assert`` or ``report`` statements, a line counts them per process: they are not
  evaluated (:ref:`cycle-changes`). The following two lines are fixed text: library warnings, and the asserts of
  the top-level unit, which are evaluated.
* ``subset`` counts the processes that fit the rules. Fewer processes join than fit when one with a rarer
  problem is missing something the lowering needs (those are the E4101 rows). A *loop group* is a set of
  processes that depend on each other in a circle without forming a combinational loop; the region evaluates
  them with more work per clock, which this count lets you see.
* ``boundary`` is the cost of the split between region and kernel. ``signal leaves exported`` are the signals the
  region drives for processes outside it (a record counts per field), and the number in brackets is how many of
  them nothing outside waits on, which makes them cheap. ``inputs`` are the signals the region reads from the
  kernel, and ``wake the region`` lists those whose change makes the region run between clock edges. When there
  are any, a ``wakes:`` line says how many of them have a combinational path to a signal that something outside
  reads (those are settled at once) and how many wait for the next clock edge.
* The last block lists the cycle-based processes, each as ``clocked`` or ``combinational``.

One process that stays out can make others expensive, because every signal between the region and the kernel is
converted and compared. If the numbers in the ``boundary`` line are large, see which process on the list is the
cause.

.. _cycle-changes:

What changes
------------

This is the whole list of differences for the processes that run cycle-based. Processes that stay in the event
simulation behave exactly as before. If a test of yours differs between ``--cycle`` and the normal mode, the cause is
here, or it is a bug in cycle mode: please report it with the smallest design that shows it.

Two-state values
~~~~~~~~~~~~~~~~

A cycle-based process computes with ``0`` and ``1`` only.

* A signal or variable without an initial value starts as ``0``, not ``'U'``. The testbench still reads ``'U'`` at
  time 0, delta 0, and ``0`` from the next delta on.
* A metavalue that reaches the region from outside (``'X'``, ``'U'``, ``'W'``, ``'-'``, ``'Z'`` and the like) is read as
  ``0``. ``'1'`` and ``'H'`` read as ``1``, ``'0'`` and ``'L'`` as ``0``. yavs counts the elements read this way and
  prints the count at the end of the run, as in the example above.
* Everything the region gives back to the kernel is ``'0'`` or ``'1'``. A testbench never sees ``'X'`` or ``'Z'``
  from a cycle-based process.
* So a bug that shows as an ``'X'`` or a ``'U'`` in the normal mode (a register without reset, an
  unconnected input, a metavalue in a comparison) cannot show. A test that checks for it passes. Run such checks
  in the normal mode. A process that compares with a metavalue literal stays out, because a two-state value never
  equals it.

``metavalue.vhd`` shows two of these: ``ticks`` has no initial value and an ``'X'`` reaches a comparison.

.. literalinclude:: examples/cycle/metavalue.vhd
   :language: vhdl
   :caption: metavalue.vhd

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

The first block is the normal mode, the second ``--cycle``. The comparison of ``a`` with ``b`` is true in cycle mode
because the ``'X'`` on ``a`` read as ``0``, and the library's warning about it is not produced either.

Reports, assertions and library warnings
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

A cycle-based process below the top-level unit does not produce ``report`` or ``assert`` messages, at any
severity, and no cycle-based process gives the library warnings of ``numeric_std`` and the other ``ieee`` packages
(metavalue and truncation warnings), also not while a call is evaluated during elaboration (below). The report counts the statements it drops per process (:ref:`cycle-report`).
The processes of the top-level unit, the testbench's, keep their checks (below).

* This includes ``severity failure``. A design that ends its own test from RTL code, with an assertion or a
  ``report ... severity failure`` that the testbench does not check, does not end it in cycle mode: the run goes on,
  and the exit status is that of however it does end. ``rtl_failure.vhd`` is such a design:

.. literalinclude:: examples/cycle/rtl_failure.vhd
   :language: vhdl
   :caption: rtl_failure.vhd

.. code-block:: text

    ** Warning: 225000000fs+0: acc reached 200
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)
    ** Failure: 275000000fs+0: acc reached 250, the design ends the test
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    ** Note: 1022000000fs+0: testbench finished, q=E8
        Process tb_rtl_failure.main
    cycle mode: 8 metavalue elements read as 0 (8 at initialisation, 0 at changes of the region's inputs)
    ** Note: 1022000000fs+0: FINISH called with status 0

In the normal mode the first block is the output and the exit status is 1, at 275 ns. In cycle mode the run reaches
the testbench's own end at 1022 ns with status 0.

* Messages of processes that stay in the event simulation are produced as before, so a testbench that checks results
  keeps working. A clocked checker of the *top-level* unit that reads only signals of the region (the design's
  outputs) runs cycle-based and evaluates its ``assert`` and ``report`` statements at the clock edge, with the
  normal mode's message text, severity, time and process name, and the same effect on the run and on the exit
  status. Its message may use string literals, ``&``, ``'image`` and ``to_string`` of integers, enumerations,
  ``std_ulogic`` and ``boolean`` values, and ``to_string``, ``to_hstring`` and ``to_ostring`` of vectors. Any other
  top-level process with an ``assert`` or ``report`` stays in the event simulation; the testbench's checks are never
  dropped. Within one delta cycle, the messages of the region come after those of the event simulation.
* The gap in this rule is a checker or monitor entity *below* the top-level unit: it looks like the design, so a
  clocked process in it joins the region and its assertions are dropped. Put such checks in a process of the
  top-level unit, or run the test in the normal mode.
* ``--assert-summary`` and ``--exit-severity`` see only the messages that are produced.
* A call of a pure function whose arguments are all known during elaboration (generics and constants:
  ``log2(DEPTH)``, a check of a register map) is evaluated once, while the design is elaborated, and its value is
  built into the region. What the design's own code in that call reports is kept: its ``assert`` and ``report``
  messages are produced once for each process holding the call, at 0 fs, in that process's name, with the normal
  mode's text. The notes and warnings of the ``ieee`` and ``std`` libraries the call gives (a truncating
  ``to_unsigned``, for example) are dropped, as everywhere else in the region. The normal mode produces them each time the statement runs (at every clock edge in a clocked
  process), so in cycle mode such a message appears once instead of at every call, at 0 fs instead of the time of
  the first call, and also when the run never takes the branch the call is in (one whose condition only the run
  decides). A call that reports with severity ``error`` or ``failure`` keeps its process in the event simulation,
  so the run stops, or counts the error, where and when the normal mode does. A call with arguments that only an
  unrolled loop or an enclosing call knows joins only when its assertions are shown to hold; otherwise its process
  stays in the event simulation. In a process of the top-level unit any such message keeps the process in the event
  simulation.
* A known gap: a message made while a constant is evaluated during elaboration (``constant c : t := f(3);`` where
  ``f`` reports) can appear twice in cycle mode (``** Warning: (init): ...`` and the like, on standard error).

Delta cycles inside the region are not visible
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The region settles all its combinational logic at once. In the normal mode a register's output appears one delta
cycle after the edge, and logic that depends on it one delta later. A testbench that looks at a signal after
``wait for 0 ns``, or in any way that depends on those delta steps, sees the settled value earlier in cycle mode.
Messages that print the delta number may show other numbers too. ``delta.vhd`` shows it in three lines:

.. code-block:: vhdl

    r <= q + 100;                 -- combinational: one delta after q
    ...
    wait until rising_edge(clk) and q = 2;
    wait for 0 ns;                -- one delta after the edge
    report "a delta after the edge: q=" & integer'image(to_integer(q)) & " r=" & integer'image(to_integer(r));

.. code-block:: text

    ** Note: 25000000fs+1: a delta after the edge: q=3 r=102
        Process tb_delta.main
    ** Note: 26000000fs+0: 1 ns after the edge:    q=3 r=103
        Process tb_delta.main
    ** Note: 26000000fs+0: FINISH called with status 0

The first block is the normal mode: ``r`` has not caught up with ``q`` yet. This is ``--cycle``:

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    ** Note: 25000000fs+1: a delta after the edge: q=3 r=103
        Process tb_delta.main
    ** Note: 26000000fs+0: 1 ns after the edge:    q=3 r=103
        Process tb_delta.main
    cycle mode: 8 metavalue elements read as 0 (8 at initialisation, 0 at changes of the region's inputs)
    ** Note: 26000000fs+0: FINISH called with status 0

Sample later: after a time step (``wait for 1 ns``), or at the falling edge. A testbench that only
reads values at a clock edge, or a time after it, gives the same result in both modes. A testbench that is correct
by timing instead of by delta order has nothing to change.

Signal history
~~~~~~~~~~~~~~

The region keeps no history of its signals. A process outside the region that reads ``'last_event``,
``'last_value``, ``'last_active``, ``'active``, ``'transaction``, ``'stable``, ``'quiet`` or ``'delayed`` of a signal
(directly, through an alias, or in a subprogram it calls) keeps the processes that drive the signal in the event
simulation (E4127), without a message: they just do not join. Under ``--cycle-strict`` it is an error, also
when the reader is the testbench. Edge tests such as ``rising_edge`` and ``'event`` are not history. Inside the
region, a process that reads the history of a signal other than its own clock stays out (E4112).

What is refused
~~~~~~~~~~~~~~~

The signals inside the region are kept in an image of their own, and nothing can observe or change them there.
So ``--cycle`` and ``--cycle-strict`` stop with E4102 before simulating when combined with ``--wave``, ``--surfer``
(which records a waveform), ``--cov``, ``--vhpi``, ``--activity`` (which counts the activations and events of every
process and signal), or a debug session (``--dap``, ``--interactive``, ``--do``). The error names the option and
nothing is written or run. It looks like this:

.. code-block:: text

    error[E4102]: `--cycle` cannot be used with `--surfer`
     |
     = note: cycle mode keeps the signals inside its region in its own image, which this option would observe or change
     = help: run without `--cycle`

    error: aborting due to 1 previous error

Run the design in the normal mode for waveforms, coverage, activity counts, debugging and VHPI plugins.
``--cycle-report`` does not simulate, so it is not refused with ``--wave``.

Options that do not read the signals of the region stay allowed: ``--profile`` (it samples the interpreter, so the
region's time shows as the kernel's and only the processes outside the region are listed), ``--assert-summary`` (it
counts the asserts that run, and cycle-based processes drop theirs), ``--shuffle``, ``--hang-report`` and
``--time-report``.

Checks that stay
~~~~~~~~~~~~~~~~

Integer overflow, subtype range checks and array index checks are still made in cycle-based processes. A failure
stops the run with the same error and exit status as in the normal mode, with the source line, the time and the
process. A check inside a function or procedure that was inlined also lists the statements it was called from, as the
normal mode does. ``overflow.vhd`` counts an integer past its largest value:

.. code-block:: text

    cycle mode: 1 of 3 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    error[E3009]: result of 2147483647 + 1 cannot be represented as INTEGER
      --> overflow.vhd:11:12
       |
    11 |       n <= n + 1;  -- overflows at the third edge
       |            ^^^^^
       |
       = note: LRM 5.2.3.1, 5.2.4.1: a predefined arithmetic operation delivers the correct result
       = note: at 25 ns, delta 0, in process tb_overflow.dut.process (overflow.vhd:8)

    error: aborting due to 1 previous error

Exit status and time
~~~~~~~~~~~~~~~~~~~~

The exit status follows the messages that are produced. A run in cycle mode ends in the same way as before when the
testbench ends it (``finish``, ``stop``, an error in a process outside the region, a failed check of a verification
library, ``--stop-time``). It ends differently only when something in the normal mode ended it from a statement
that cycle mode drops (a failure raised by RTL code), or when a signal that the testbench waits for is seen at
another delta. See :ref:`cycle-troubleshooting`.

.. _cycle-strict:

Strict mode
-----------

``--cycle-strict`` is for a design that should be cycle-based as a whole, for example in CI to notice when a
change moves a process out of the region. The selection is the same as with ``--cycle``. The difference is that a
process that is RTL by shape and cannot join is an error, with the code of the reason (E4101, E4110 to E4127), all of
them listed, and the run stops with exit status 2 before simulating.

Two groups are exempt and never an error:

* every process of the top-level design unit, the testbench itself;
* *behavioural* processes anywhere in the design: timed waits, ``wait;``, several waits, files and text input and
  output, access values, protected objects. A bus model below the top-level unit is of this kind.

``left_out.vhd`` under strict mode. The errors are printed in this order; the second and later ones look
like the ones shown, and the notes under each are the same:

.. code-block:: bash

    yavs run --cycle-strict --work=work:build/work tb_left_out

.. code-block:: text

    error[E4127]: process tb_left_out.dut.q <= ... (left_out.vhd:26) is outside what cycle mode can run
      --> left_out.vhd:77:43
       |
    77 |     report "q last changed " & time'image(q'last_event) & " ago";
       |                                           ^^^^^^^^^^^^ tb_left_out.main reads `'last_event` of tb_left_out.q, and the region keeps no history of the signals it drives: its drivers stay in the event kernel
       |
       = note: `--cycle-strict` needs every process below the top-level design unit that is not behavioural to run cycle-based
       = help: run with `--cycle` to leave it in the event kernel

    error[E4119]: process tb_left_out.dut.p_latch is outside what cycle mode can run
      --> left_out.vhd:31:5
       |
    31 | /     if en = '1' then
    32 | |       held <= d;
    33 | |     end if;
       | |___________^ it does not assign tb_left_out.held(0) on every path, so it keeps its value: a latch (this statement leaves it unassigned on a path)
       |
       = note: `--cycle-strict` needs every process below the top-level design unit that is not behavioural to run cycle-based
       = help: run with `--cycle` to leave it in the event kernel

    ...

    error: aborting due to 4 previous errors

The errors you meet most, and what to do:

.. list-table::
   :header-rows: 1
   :widths: 14 86

   * - Code
     - Fix
   * - E4119
     - A latch. Assign the signal on every path of the process: give it a default value at the top of the
       process, before the ``if``.
   * - E4121
     - Incomplete sensitivity list. Use ``process (all)`` (VHDL-2008), or list every signal the process reads.
   * - E4120
     - A variable of a combinational process is read before it is assigned, so it is state. Assign it first.
   * - E4124
     - Two processes drive the same signal element. Drive it from one process.
   * - E4111
     - ``after`` inside RTL. Delays belong in testbench models; remove it from the design, or keep the model out
       of the design hierarchy.
   * - E4123
     - A clocked process of another shape. Write it as ``if rising_edge(clk) then ... end if;`` with an
       optional asynchronous reset branch before it, and nothing else in the process.
   * - E4112, E4127
     - A history attribute. Replace it with an edge test or a register; for E4127 let the testbench keep its own
       copy of the value instead of reading the history of the design's signal.
   * - E4113, E4118
     - ``real`` or ``time`` values computed at run time, and ``'Z'``. These cannot join; use ``--cycle`` and
       accept that they stay in the event simulation, or move the model into the testbench.
   * - E4101
     - Not implemented yet. The process is in the subset, and a later version may take it. Use ``--cycle`` meanwhile.

The other codes are in :doc:`error-codes`, and ``yavs --explain E4119`` prints the whole explanation of one.

.. _cycle-cache:

Start-up cost and the cache
---------------------------

yavs selects and analyses the region at the start of every run, and Cranelift compiles its functions the first
time. The time that takes grows with the size of the region, not with the length of the simulation: for a short test
of a large design it can be longer than the time the faster simulation saves.

The compiled machine code is kept in the compilation cache of the work library (``yavs-cache/cycle`` next to the
library, :doc:`on-disk`). A later run that makes the same functions links them instead of compiling them. In a test
suite, the test cases of one design and the same generics therefore compile the region once. An edited file, other
generics, other options that change the design, or another yavs build make new entries. The size of the store is
limited (16384 functions, 512 MiB; a region of more functions than that is not cached at all), and ``yavs cache clean --work=NAME:PATH`` empties it. ``--no-cache`` makes a run neither read nor write it,
so such a run compiles every time.

When several tests start at once on a cold cache, each of them compiles the region. Run one test of the suite first, or
keep ``yavs-cache`` between CI runs (:doc:`ci`), so that the others find the code.

.. _cycle-troubleshooting:

Troubleshooting
---------------

**The results differ from the normal mode.** Go through the list:

1. Does the end of the run say that metavalues were read as ``0``? Then ``'U'``, ``'X'`` or an uninitialised
   register mattered. Fix the design or the test, or check it in the normal mode.
2. Does ``--cycle-report`` list asserts and reports as not evaluated? Then RTL code that used to report, fail, or end
   the test does not do it now (:ref:`cycle-changes`).
3. Does the testbench read a value a delta after the edge, or depend on delta numbers? Sample later.
4. Does the testbench read a signal that has no initial value at time 0? It reads ``'U'`` at delta 0 in both modes
   and ``0`` from the next delta on in cycle mode.

If none of these explains it, it is a bug in cycle mode: a wrong result is never an accepted loss. Report it with
the smallest design that shows it.

**A test hangs, or ends at another time.** If the end of the test came from RTL code (an ``assert`` or ``report`` of
severity failure in the design, which is dropped), the run now goes on to the testbench's own end or to
``--stop-time``. Give the test a ``--stop-time``, or move the check into the testbench. If it is a testbench that waits
for a signal in the same delta as a register, see delta cycles above. ``--cycle-report`` shows whether the process
that raises the message joined.

**A design is slower, or not faster.** Read the first line of the report:

* Few processes joined: the E4101 rows and the ``boundary`` line say what keeps the rest out. A clocked checker of the
  top-level unit that holds an ``assert`` stays in the event simulation by design, and so does anything whose history
  the testbench reads.
* The testbench is most of the run: ``--profile`` shows where the time goes (:doc:`running-faster`). Cycle mode does
  not touch that part.
* The test is short: see the start-up cost above, and keep the cache between runs.

See also
--------

* :doc:`cli-reference`: ``--cycle``, ``--cycle-strict`` and ``--cycle-report``.
* :doc:`vunit-yavs`: the ``yavs.cycle`` sim option.
* :doc:`error-codes`: the codes E4101 to E4127.
* :doc:`running-faster`: the other ways to a faster run.
* :doc:`environment`: the ``YAVS_CYCLE_*`` variables for developers.
