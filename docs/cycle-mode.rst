Cycle mode
==========

Cycle mode runs the synchronous RTL of a design as compiled code that is evaluated once per clock edge, and leaves
the testbench in the ordinary event simulation. You turn it on per run with ``--cycle``; without it nothing changes.
It trades some of the simulator's detail for speed, and this page lists what it trades, so that you can decide
whether your tests can live with it.

.. warning::

   Cycle mode is **experimental**. It is opt-in, and nothing changes without ``--cycle``. Its options
   (``--cycle``, ``--cycle-strict``, ``--cycle-report``, ``--cycle-exact-meta``, the vunit-yavs sim option
   ``yavs.cycle``), its error codes, the processes it accepts and what it gives up may change in a later release
   without a deprecation period. The set of designs it accepts is still growing: a process it cannot take yet simply
   stays in the event simulation (see :ref:`cycle-report`).

   Please `report <https://github.com/ru551n/yavs/issues>`__ a test that passes without ``--cycle`` and fails or
   ends differently with it, beyond what this page lists, and an internal error (``E9999``). Attach the design or
   the smallest part of it that shows the difference, the command line and the output of ``--cycle-report``.

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
At a clock edge only the logic that the registers of that clock and the changed inputs reach runs, and only the
outputs that logic writes are compared with what was last driven, so in a design with several clocks an edge of one
clock costs what that clock's part of the design costs. A small region whose last edge changed nothing, with no
input changed since, skips the next edge of the same clocks, as the event simulation skips an idle process. An event of
a clock that is not an edge the region uses does not wake it, also when the region waits on data inputs as well.

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

``--cycle-exact-meta``, given with any of the three, keeps the testbench's metavalues exact and the values in the
message of a failed check (:ref:`cycle-contract`):

.. code-block:: bash

    yavs run --cycle --cycle-exact-meta --work=work:build/work tb_counter

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
simulate, so that test fails on purpose. ``yavs.sim_flags`` ``["--cycle-exact-meta"]`` with ``yavs.cycle`` on gives a
test the exact metavalue rules. :doc:`vunit-yavs` has the option in its table.

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
is the first sign of what changes: nine elements of signals that start as ``'U'`` were read as ``0``. It counts
reads, not signals: an element read as a metavalue at two wakes of the region counts twice, and a metavalue that comes
and goes between two reads is not counted (an input that only clocked processes read is read at the region's wakes).

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
``std_logic_1164`` operators (a shift operator such as ``sll`` by a count computed at run time only when the count's
subtype cannot be negative, such as ``natural``), qualified record aggregates whose fields provably fit their
subtypes, ``case``, ``if``, ``for`` loops whose bounds are known when the design is elaborated
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
   * - Is a register whose asynchronous reset another register makes, and whose output is the clock or the
       asynchronous reset of a process: the normal mode can change it at the edge and again a delta later, and
       that pulse resets or clocks the other process
     - E4132. The region settles the edge and the reset together and would not show the pulse.
   * - Is combinational logic, or a register with an asynchronous reset (the normal mode can change it twice in
       one time step: at the edge and at a reset made by another register), that feeds, back to the registers
       (processes that act at clock edges alone), a process outside the region whose result may depend on the delta cycles its inputs change in: one that
       keeps a value of an earlier run (a combinational loop included, such as ``l <= d when en = '1' else
       l;``), or, below the top-level unit, a process that drives a signal and has an incomplete sensitivity
       list or that cycle mode could not show keeps no value (a process with a ``'Z'`` and a variable, for
       example). A concurrent assignment without a delay, with a final ``else``, that reads nothing it drives
       and stays out for its ``'Z'`` alone does not count (one with ``after``, ``transport`` or ``reject``
       does: a one-delta glitch in the normal mode removes or adds a transaction), nor does a process that acts at clock edges alone
       (logic between clocks whose edges come in different delta cycles is the row below). Testbench code (the top-level unit, and a monitor that drives no signal) sees the settled values
       earlier, as described below
     - E4129. The region settles such logic in one delta cycle where the normal mode takes several, so that
       process would see other values or run another number of times.
   * - Shares a signal element with another process, or the signal has a resolution function of its own
     - E4124.
   * - Is combinational logic between the registers of two clocks whose edges may come in different delta
       cycles of one time step (a clock gated or computed by an assignment comes one delta cycle after its
       source; a plain copy is its source, :ref:`cycle-clock-copies`). The edges
       of clocks made by assignments with a delay (``clk <= not clk after 5 ns;``) all come in the first delta
       cycle of their time step, and those of clocks made by clock processes that wait for a time and assign
       without a delay in the second, when yavs knows the delays and times when the design is elaborated:
       logic between clocks of one of these kinds joins, and so does logic that registers of the second kind
       (or of a clock process whose time yavs does not know, whose edges come in the second delta cycle or
       later) feed and registers of the first or second kind sample. The path counts whatever lies on it: processes outside the
       region for any reason (a testbench process excepted), registers outside the region, ports. Logic that
       computes a clock from what the region drives stays out too: the clock's edge would come early
     - E4130, and E4131 for logic that computes a clock. The normal mode samples such logic before its new value
       arrives; the region would hand over the settled value.
   * - Is clocked in a form that is not one of those above, or uses a clock made inside the region
     - E4123, E4125.
   * - With ``--cycle-exact-meta``: assigns a metavalue literal to a signal the testbench reads, or passes on a
       ``'Z'`` from outside to such a signal
     - E4137.
   * - Uses an external name, or a loop whose bounds are not known when the design is elaborated, that is too
       large to unroll, or that leaves an outer loop with ``exit`` or ``next``
     - E4138, E4139.
   * - Drives part of a signal whose other elements a process outside the region drives, a signal that also gets
       a value from a port connection or a constant, or a memory written in its reset branch, at fixed and
       run-time indices both, or read outside the region; or drives a signal a ``force`` or ``release`` of the design can change
       (every process, when the force names a signal parameter)
     - E4134, E4135, E4133, E4128: limitations of yavs.
   * - Compares with a metavalue literal (``= 'U'``), has a ``while`` loop, or another construct cycle mode cannot
       take *yet*
     - E4101: in the subset, not implemented yet. This is the reason that is expected to shrink. E4126 is the same
       for a construct yavs does not follow far enough to show the process is logic.

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
      asserts and reports are evaluated: at the clock edge or after the settle in region code, else the process stays in the event kernel
      subset: 1 of 7 processes fit, 0 loop groups among them
      boundary: 1 signal leaves exported (0 nothing outside waits on), 1 inputs, 0 wake the region
      cycle-based, 1 process:
        tb_left_out.dut.p_reg (left_out.vhd:20)     clocked

* The first line says how many processes join and on which clock edges they run.
* Then come the processes that stay, grouped by reason, the most common reason first. A row has the process (an
  instance path and where it starts) and the reason with the position of the construct that causes it, in the
  same words as the error of ``--cycle-strict``: the construct, and after a colon what at that place causes it. A
  concurrent assignment is shown as ``name <= ...``. A delayed clock like ``clk <= not clk after ...`` is on
  almost every list and is expected. *Behavioural* processes are the ones that are testbench by nature.
* Under ``--no-asserts``, a line counts the ``assert`` and ``report`` statements cycle-based processes drop, per
  process (:ref:`cycle-no-asserts`). The next block lists each statement once (one in a loop counts once per
  process here), failures first: the severity, where it is, and the process. The following two lines are fixed
  text: library warnings, and how asserts and reports are evaluated.
* ``subset`` counts the processes that fit the rules. Fewer processes join than fit when one with a rarer
  problem needs something yavs does not implement yet, or a rule of the region as a whole keeps it out (the
  rows of E4101 and E4128 to E4137). A *loop group* is a set of
  processes that depend on each other in a circle without forming a combinational loop; the region evaluates
  them with more work per clock, which this count lets you see.
* ``boundary`` is the cost of the split between region and kernel. ``signal leaves exported`` are the signals the
  region drives for processes outside it (a record counts per field), and the number in brackets is how many of
  them nothing outside waits on, which makes them cheap. ``inputs`` are the signals the region reads from the
  kernel, and ``wake the region`` lists those whose change makes the region run between clock edges. When there
  are any, a ``wakes:`` line says how many of them have a combinational path to a signal that something outside
  reads, to a check, or to the asynchronous reset of a register (those are settled at once) and how many wait for
  the next clock edge.
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

.. _cycle-contract:

The contract in short
~~~~~~~~~~~~~~~~~~~~~

Cycle mode is for speed. By default it gives up these, and nothing else, in the processes that run cycle-based:

* **Metavalues.** ``'U'``, ``'X'``, ``'Z'``, ``'W'`` and ``'-'`` read as ``0`` and ``'H'`` as ``1`` where they reach
  the region, and the region gives back only ``'0'`` and ``'1'``, also to the testbench: a testbench that expects an
  ``'X'`` or a ``'Z'`` from the design sees ``'0'`` (:ref:`cycle-two-state`). ``--cycle-exact-meta`` keeps the
  processes that would hide a metavalue from the testbench in the event simulation instead.
* **Delta order.** The region settles its logic at once, and a clock copied by an assignment is its source, so a
  register on the copy samples the source's registers before the edge where the normal mode, a delta cycle later,
  gives their new values (:ref:`cycle-deltas`, :ref:`cycle-clock-copies`).
* **Library warnings.** The ``numeric_std`` and other ``ieee`` warnings (metavalues, truncation) are not given by
  cycle-based processes. Their own reports and assertions are evaluated, on settled values, so a combinational
  process reports once per time step where the normal mode may report each delta (:ref:`cycle-reports`).
* **Values in error messages.** A failed check (overflow, range, index) stops the run with the normal mode's error
  code, source line, time and process, and says what failed, but not the values involved: keeping them costs code
  in every check. ``--cycle-exact-meta`` keeps them, with the normal mode's message (:ref:`cycle-checks`).

.. _cycle-two-state:

Two-state values
~~~~~~~~~~~~~~~~

A cycle-based process computes with ``0`` and ``1`` only.

* A signal or variable without an initial value starts as ``0``, not ``'U'``. The testbench still reads ``'U'`` at
  time 0, delta 0, and ``0`` from the next delta on.
* With ``--cycle-exact-meta`` the region follows those start values for a while: it runs the same code a second
  time on a second set of values, where every ``'U'`` at the start (and every metavalue that reaches an input in that
  time) is ``1`` instead of ``0``. An output bit that differs between the two depends on a start value, and the
  testbench reads ``'U'`` there, as in the normal mode (a register without reset shows ``'U'`` until it is first
  written, and the event from ``'U'`` to ``0`` at time 0 is gone). It ends, and the outputs are ``0`` and ``1`` again,
  when the two agree on everything the region keeps, after 1024 clock edges (of any clock) at the most, or when a check (a range,
  an index) fails on the second set of values only, which never stops the run. The summary line says how it ended:

  ``metavalue.vhd`` below shows it.

  It is not the normal mode's ``'U'``: a bit is ``'U'`` when its value depends on a start value, so ``q and not q``
  gives ``0`` where the normal mode gives ``'U'``. What is followed:

  - signals, registers, variables of clocked processes and the words of memories without an initial value (also
    a memory the testbench reads: a word nothing wrote shows ``'U'``), and the variables of a called subprogram
    without one (``'U'`` at every call);
  - a metavalue that reaches an input while the second image runs. A bit that depends only on an ``'X'``, ``'Z'``,
    ``'W'`` or ``'-'`` shows ``'X'``; one that needs a ``'U'``, alone or together with one of those, shows ``'U'``,
    as ``std_logic`` gives ``'U'`` for such a mix. To tell them apart, two more sets of values run from the first
    such metavalue on (one with only the ``'U'`` ones at 1, one with only the others);
  - choices and conversions on such a value in a combinational process: the second set takes the branch the
    region's own values take (``=``, ``/=``, ``if s then``, ``case``, ``std_match``, ``numeric_std``
    comparisons, ``to_integer`` as an index, ``to_01`` and ``to_bit``). Where the region's choice is the normal mode's (``s = '1'`` is false for a
    ``'U'``, ``to_integer`` of a ``'U'`` address is 0 and reads element 0), the result is the normal mode's.
    When a value compared there is not the same on the second set (it depends on a ``'U'``), the region's choice
    may not be the normal mode's, so everything the process drives takes the region's values on the second set
    too: such a combinational process shows no ``'U'``, also where the normal mode has one. This holds where the
    value compared is a signal (a register, a port, a field or slice of one); a comparison of a variable, of a
    value computed first, or of an element chosen at run time takes the branch its own value gives (``?=`` too,
    which gives ``'U'`` in the normal mode).
  - Integers have no ``'U'``: they start at ``'left`` in both sets of values, as in the normal mode.

  Where it still differs from the normal mode:

  - An ``assert`` or ``report`` of the region formats the region's own values: a register's ``'U'`` start reads
    ``0`` there. A process whose assert or report reads a signal the region does not drive alone stays in the event
    simulation (:ref:`cycle-reports`), so only what members drive is seen this way.

  - The region's own set of values reads a ``'U'`` as ``0``. Where that takes the branch the normal mode does not
    (``y <= a when s = '0' else b`` with ``s`` ``'U'``: the normal mode gives ``b``), the result is the region's
    (``a``), as without the second image.
  - A clocked process makes its choices on each set's own values: a choice on a ``'U'`` may show ``'U'``
    where the normal mode has a value (``if s = '1' then q <= u; else q <= b; end if;`` with ``s`` and ``u``
    ``'U'``: the normal mode gives ``b``), and an index of a ``'U'`` address may fail its range check on the
    second set, which then ends. Taking the region's choices there instead keeps a ``'U'`` that comes from a
    memory read at a ``'U'`` address (as the normal mode does), which then reaches such choices further on: in a
    CPU this made a ``'U'`` in its cache the normal mode does not have.
  - A choice on a variable or a value computed first (not a signal) may differ between the two sets: then the
    bits it decides show ``'U'`` even where the normal mode has a value.
  - So a clocked process with a choice keeps a
    register's value where both sets agreed and only the second changed it, as the normal mode keeps a register
    whose enable is ``'U'``; a register of it that takes a ``'U'`` from its data while keeping its value in the
    region's own set then reads ``0`` or ``1``.
  - ``numeric_std`` arithmetic on a metavalue gives all ``'X'`` in the normal mode; here only the bits that
    depend on it show ``'U'`` or ``'X'``.
  - A ``'U'`` the region drives that comes back to an input through a process outside the region reads as ``0``
    in the region's own set of values, as any metavalue there, where without the second image that process
    would have passed on the region's ``0`` or ``1``. So the region's values may differ from a run without
    ``--cycle-exact-meta``'s second image. No rule can avoid it: the process outside may pass the value on as it
    is or inverted, and both give ``'U'``.

  The default ``--cycle`` has none of this and pays nothing for it.
* A metavalue that reaches the region from outside (``'X'``, ``'U'``, ``'W'``, ``'-'``, ``'Z'`` and the like) is read as
  ``0``. ``'1'`` and ``'H'`` read as ``1``, ``'0'`` and ``'L'`` as ``0``. yavs counts the elements read this way and
  prints the count at the end of the run, as in the example above.
* So a change between two values with the same two-state value (``'1'`` to ``'H'``, ``'U'`` to ``'X'``) is no change in
  the region: a combinational process of the region does not run for it and makes no ``report`` or ``assert``
  message where the normal mode makes one.
* So a cycle-based process that copies a line another driver pulls up (a released ``'H'``) to a signal the testbench
  reads gives the testbench ``'1'`` where the normal mode gives ``'H'``. ``to_x01`` makes ``'1'`` of both; a test
  that compares the signal with ``'1'`` or ``'H'`` directly sees the difference.
* Everything the region gives back to the kernel is ``'0'`` or ``'1'``. A testbench never sees ``'X'`` or ``'Z'``
  from a cycle-based process: a process that assigns a metavalue literal (``(others => 'X')`` on invalid data, say)
  gives ``'0'`` there, and so does one that copies a tri-state line some process outside sets to ``'Z'``. A metavalue
  literal reads as ``0``, as do values computed from it.
* With ``--cycle-exact-meta`` the testbench sees what the normal mode gives it in those two cases: a process that
  assigns a metavalue literal to a signal the testbench reads (a process of the top-level unit, or a behavioural
  one) stays out, and so does one that passes on to such a signal a signal a process outside sets to ``'Z'`` in its
  own statements, or that any process outside but a behavioural one passes on in a value it assigns (a copy of a
  tri-state line, also one that stays out for another reason, such as a delay; a line another driver pulls up never
  holds a ``'Z'``; a ``'Z'`` made in a called subprogram, given by a generic or passed on by a behavioural process is
  not seen). The price is speed: those processes, and often their neighbours, run in the event simulation.
* An ``'X'`` or ``'U'`` that a process outside the region sets is read as ``0`` as well, also when a cycle-based
  process copies it on to a signal the testbench reads, with ``--cycle-exact-meta`` too (there it shows as ``'X'``
  or ``'U'`` while the second image runs, then as ``'0'``). Such a copy joins the region, so the
  testbench sees ``'0'`` where the normal mode gives ``'X'``. A bus model that checks the read data it is handed
  against an expected ``'X'`` (a read of a location nothing wrote, say) reports "got ``0``, expected ``X``" under
  ``--cycle``; run such tests in the normal mode.
* A signal the region reads that nothing drives keeps its start value for the whole run. When that is a metavalue
  (``'U'`` of a ``std_logic`` without an initial value), the normal mode reads it all the time and the region reads
  ``0``. A test that worked only because of the metavalue (a ``valid`` nobody drives, read as "not ``'0'``") then runs
  differently, and may never end. The summary line counts such inputs and ``--cycle-report`` names them:

  .. code-block:: text

      cycle mode: 359 of 488 processes run cycle-based, 129 in the event kernel, 3 inputs driven by nothing read as 0 (see --cycle-report)

  .. code-block:: text

      3 signals the region reads without a driver, each keeping its metavalue start value for the whole run (the region reads 0):
        prim32_tc.bus_wrep_valid
        prim32_tc.bus_wrep_ok
        prim32_tc.num_dvalid

  An unused field of a record port that nothing drives is counted too; it changes nothing.
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

With ``--cycle --cycle-exact-meta`` the second image keeps ``ticks`` at ``'U'`` as the normal mode does. ``eq`` is
``1``, as without it: the region compares ``0000`` with ``0000`` (the ``'X'`` on ``a`` read as ``0``); the
second set compares ``1111`` and would give ``'0'``, but the clocked process keeps the register where only the
second set changed it (see above). The normal mode gives ``0``:

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

.. _cycle-reports:

Reports, assertions and library warnings
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Cycle-based processes evaluate their ``assert`` and ``report`` statements, with the normal mode's message text,
severity, time and process name, and the same effect on the run: an ``error`` counts for the exit status, a
``failure`` (or whatever ``--exit-severity`` names) ends the run at the time the normal mode ends it.

* A clocked process evaluates them at its clock edge, on the values before the edge, as the normal mode does.
* A combinational process evaluates them once its inputs are settled, and only in a wake where an element it waits
  on (its sensitivity list, or what it reads for ``process (all)`` and a concurrent ``assert``) changed since its
  last run: the event that runs it in the normal mode. The region computes its logic in dependency order, so the
  values an assert sees are the settled values of that time step, never a value the region passes through on the
  way: a process runs after every process that makes what it reads, its asserts included, and in a loop group the
  runs before the last make no messages.
* No cycle-based process gives the library warnings of ``numeric_std`` and the other ``ieee`` packages (metavalue
  and truncation warnings), also not while a call is evaluated during elaboration (below).

What can differ:

* **Delta and order.** A message of the region carries the delta of the region's wake; in the normal mode a
  message of combinational logic comes some deltas later. Within one run of the region the clocked processes'
  messages come first, then the combinational ones', each in elaboration order. Against the messages of the
  processes in the event simulation they come in the order of the wakes: within one time step a message of the
  region may come before or after one the normal mode prints before or after it.
* **Intermediate values.** In the normal mode a combinational process runs again in every delta where one of its
  inputs changes, so it can report a value its inputs hold for one delta only, or report twice in one time step.
  In cycle mode it runs once per wake on settled values: such a message is made once, with the settled values, or
  not at all when the settled values satisfy the assert.
* **Metavalues.** Under the default two-state values an assert sees ``'U'`` and ``'X'`` as ``0``
  (:ref:`cycle-two-state`). An assert that fires in the normal mode only because a value is still ``'U'`` (a
  register before its reset, ``unsigned(addr) < DEPTH`` on an uninitialised address) does not fire in cycle mode,
  and one that holds only because of a metavalue may fire. A combinational process makes its messages when what
  it waits on changed as two-state values: a change from ``'1'`` to ``'H'`` or from ``'U'`` to ``'X'`` is none.
  Under ``--cycle-exact-meta`` a process with an ``assert`` or ``report`` that reads a signal the region does not
  drive alone stays in the event simulation (E4140), so it sees the metavalues the rest of the design gives it;
  what members drive it still sees two-state (a register's ``'U'`` start as ``0``).

An assert or report cycle mode cannot evaluate keeps its process in the event simulation, where it runs as before
(E4140 in the report, an error below the top-level unit under ``--cycle-strict``). Cycle mode takes a message made of
string literals, ``&``, ``'image`` and ``to_string`` of integers, enumerations, ``std_ulogic`` and ``boolean``
values, and ``to_string``, ``to_hstring`` and ``to_ostring`` of vectors, with a severity known during elaboration.
A concurrent ``assert not (rising_edge(clk) and a and b)`` (any edge test as the first operand, any number of
operands after it) is a check at that clock edge, on the values before it, as its process in the normal mode does
nothing between edges. It does not take an assert in the asynchronous reset branch of a clocked process, a
concurrent ``assert`` that tests a clock edge in another form (``assert not rising_edge(clk) or ...``), or a
combinational process whose sensitivity list names a signal it does not read. An assert that holds by what is known
during elaboration (a width check of a called function on its constant arguments) is no code at all.

.. _cycle-no-asserts:

Dropping the design's asserts: ``--no-asserts``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

``--no-asserts`` drops the ``assert`` and ``report`` statements of the processes below the top-level design unit, in
the normal mode and in cycle mode alike, for speed. The processes of the top-level unit (the testbench) keep
theirs, and the statements inside subprograms run wherever they are called from, so a testbench's checks and the
check and logging calls of its verification components still run. Below the top-level unit is everything the
testbench entity instantiates: the design, and verification components, bus models and checker entities. The run
starts with a note that counts the statements it drops; ``--cycle-report`` lists them per process with their
severity. A process kept out only by an assert it cannot evaluate joins the region under ``--no-asserts``.

.. note::

   A design that ends its own test with an ``assert`` or ``report`` below the top-level unit runs on under
   ``--no-asserts``: until ``--stop-time``, or until the testbench ends the run.

``rtl_failure.vhd`` is such a design: it ends its test from RTL code, with a ``report ... severity failure`` that the
testbench does not check.

.. literalinclude:: examples/cycle/rtl_failure.vhd
   :language: vhdl
   :caption: rtl_failure.vhd

In the normal mode and with ``--cycle`` the run ends at 275 ns with exit status 1:

.. code-block:: text

    ** Warning: 225000000fs+0: acc reached 200
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)
    ** Failure: 275000000fs+0: acc reached 250, the design ends the test
        Process tb_rtl_failure.dut.process (rtl_failure.vhd:16)

``--cycle`` prints on standard error:

.. code-block:: text

    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    cycle mode: 8 metavalue elements read as 0 (8 at initialisation, 0 at changes of the region's inputs)

With ``--no-asserts`` (in either mode) both statements are dropped, the run starts with this note on standard error:

.. code-block:: text

    note: `--no-asserts`: 2 assert and report statements in 1 process below the top-level unit are not evaluated

and it reaches the testbench's own end at 1022 ns with status 0:

.. code-block:: text

    ** Note: 1022000000fs+0: testbench finished, q=E8
        Process tb_rtl_failure.main
    ** Note: 1022000000fs+0: FINISH called with status 0

* Messages of processes that stay in the event simulation are produced as before. A process of the *top-level*
  unit with an ``assert`` or ``report`` joins the region only when it is clocked and reads nothing but signals the
  region drives (the design's outputs): its checks then see exactly the values the normal mode's would, at the
  clock edge. Any other, a combinational one included, stays in the event simulation; the testbench's checks are never dropped, also not under ``--no-asserts``.
* ``--assert-summary`` and ``--exit-severity`` count the messages of the region as those of the normal mode.
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
* A message made while a constant is evaluated (``constant c : t := f(3);`` where ``f`` reports, ``** Warning:
  (init): ...`` on standard error) appears once, as in the normal mode, also when every process of the constant's
  region runs cycle-based. ``--cycle-report`` does not run the design and does not show these messages. A known
  gap: when every process of the region runs cycle-based and the constant's function has no ``report`` or ``assert``
  of the design's own, the region's constants are not evaluated in the run, so a warning of the ``ieee`` libraries
  they give (a truncating ``to_unsigned``) is not made, and one that fails (an overflow) does not stop the run.

.. _cycle-deltas:

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

.. _cycle-clock-copies:

Clocks copied by an assignment
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

A clock copied without change is the same clock as its source in cycle mode: a concurrent assignment without a delay
of the clock's name (``clk_b <= clk;``), of an aggregate of clock names (``clks <= (2 => clk_cpu, others =>
clk_ctrl);``, also chosen by a condition the design's generics decide), or a port the clock is connected to. The
registers on the copy run at the source's edge, together with the source's registers, on the values before that edge,
and the logic between them joins the region. In the normal mode the copy changes one delta cycle after its source, so
there:

* a register on the copy that samples logic of the source's registers gets the value before the edge, as in cycle
  mode;
* a register on the copy that reads a register of the source directly gets the *new* value, which the source's
  register took one delta cycle earlier. Cycle mode gives the value before the edge, as the hardware does. A design
  that works only in the normal mode because of that delta (or that adds a signal assignment as a delay of one delta
  cycle to make up for it) can give other values in cycle mode.

A clock computed from another (``not clk``, ``clk and en``, a divider) is no copy: it keeps its own edges, and logic
between it and its source stays in the event simulation as before. Nor is a copy whose start value would give it an
edge at time 0 that its source does not have (a ``clk'event and clk = '1'`` user of a copy of a clock that starts at
``'1'``). When a process in the event simulation has a clock edge on the copy, the cycle-based processes on that
copy keep the copy's own edges too, so that process samples them as in the normal mode.

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

.. _cycle-checks:

Checks that stay
~~~~~~~~~~~~~~~~

Integer overflow, subtype range checks and array index checks are still made in cycle-based processes. A failure
stops the run with the same error code and exit status as in the normal mode, with the source line, the time and the
process. The message says what failed (the operator of an overflow, the subtype and range of a range check, the
range of an index), but not the values: a failed check stores nothing, so each check is less code. With
``--cycle-exact-meta`` the checks keep the values and the message is the normal mode's. A division by zero and a
quotient too large for its type are one message. A check inside a function or procedure that was inlined also lists the statements it was called from, as the
normal mode does. The parameters of a called procedure are checked as in the normal mode: a value driven through a
scalar signal parameter against the parameter's subtype and the actual signal's, an ``inout`` variable at the call
and the value copied back to a variable against the variable's subtype. A parameter or a subtype whose range reads a
variable keeps its process in the event kernel (E4101): the range has the value of the variable where it is declared,
which cycle-based code does not keep. With several clocks, the processes of each clock edge run together, one clock after the other:
when checks of two clocks fail at the same edge, the run stops with that of the clock that runs first, which need
not be the one the normal mode reports. In the same way a message of a top-level checker on one clock can come before
the failure of a process on another clock at that edge. ``overflow.vhd`` counts an integer past its largest value:

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

Exit status and time
~~~~~~~~~~~~~~~~~~~~

The exit status follows the messages that are produced. A run in cycle mode ends in the same way as before when the
testbench ends it (``finish``, ``stop``, an error in a process outside the region, a failed check of a verification
library, ``--stop-time``). It ends differently only when something in the normal mode ended it from a statement
that ``--no-asserts`` drops (a failure raised by RTL code), or when a signal that the testbench waits for is seen at
another delta. See :ref:`cycle-troubleshooting`.

.. _cycle-strict:

Strict mode
-----------

``--cycle-strict`` is for a design that should be cycle-based as a whole, for example in CI to notice when a
change moves a process out of the region. The selection is the same as with ``--cycle``. The difference is that
every reason a process that is RTL by shape cannot join is an error, and the run stops with exit status 2 before
simulating.

Two groups are exempt and never an error:

* every process of the top-level design unit, the testbench itself;
* *behavioural* processes anywhere in the design: timed waits, ``wait;``, several waits, files and text input and
  output, access values, protected objects. A bus model below the top-level unit is of this kind.

Each error says, in this order:

* **what** is not supported, in its first line: ``error[E41xx]: <construct> is not supported in cycle mode``;
* **where**: the statement that causes it, with what there causes it under the marker, and for a situation that
  spans processes (a delta race between clocks, a reader outside the region, a second driver) a second marker at
  the other process or signal; an error inside a called subprogram marks the call too;
* the processes it keeps out, with their instance paths (``in process ...``, or ``in 7 processes: ...`` for a
  statement of an entity used several times);
* **why** it cannot be cycle-based (``note: why:``);
* **how to fix** it (``help:``), or that it is a limitation of yavs and not a problem in the design.

There is one error per code and statement, sorted by file and line. At most 40 are printed; a last line says
how many more there are, and ``--cycle-report`` lists them all.

``left_out.vhd`` under strict mode:

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

Every code, what it means and what to do. *Limitation* marks a reason that is a gap in yavs, not a problem in the
design: the process is fine for the normal mode, and ``--cycle`` leaves it in the event simulation.

.. list-table::
   :header-rows: 1
   :widths: 10 35 55

   * - Code
     - Not supported
     - Fix
   * - E4101
     - A construct yavs does not implement in cycle-based code yet (the headline names it)
     - Limitation. Write the construct another way (the label often says how), or use ``--cycle``.
   * - E4110
     - A wait other than ``wait until <edge>;`` as the first statement
     - Write a clocked process (``if rising_edge(clk) then ... end if;``) or a combinational one with a
       sensitivity list; a model with waits belongs in the testbench.
   * - E4111
     - ``after``, ``transport``, ``reject``, a waveform of several elements
     - Assign without a delay in the design; keep delays in testbench models.
   * - E4112
     - Signal history (``'last_value``, ``'stable``, ``'event`` outside the clock's edge test, an edge test of
       another signal)
     - Use an edge test of the process's own clock, or keep the old value in a register.
   * - E4113
     - A ``real`` or ``time`` value computed at run time
     - Use an integer or a constant, or move the model into the testbench.
   * - E4114
     - Shared variables used by several processes, protected objects, access values
     - Pass values between processes through signals; keep the rest in the testbench.
   * - E4115
     - Postponed processes, guarded blocks, signals and assignments, ``null`` waveforms
     - Use a plain process and an ordinary signal with one driver.
   * - E4116
     - ``force`` and ``release`` in a process of the design
     - Force signals from the testbench.
   * - E4117
     - A call of a subprogram without a VHDL body, or one that changes what it is not given
     - Give it a body; in a combinational process, pass what it changes as parameters.
   * - E4118
     - The value ``'Z'``
     - Keep tri-state buses in the testbench, or drive ``'0'`` and ``'1'`` with a separate enable.
   * - E4119
     - A latch: a signal not assigned on every path of a combinational process
     - Give it a default value at the top of the process, before the ``if``.
   * - E4120
     - A variable of a combinational process read before it is assigned
     - Assign it at the start of the process.
   * - E4121
     - An incomplete sensitivity list
     - Use ``process (all)``, or list every signal the process reads.
   * - E4122
     - A combinational loop
     - Break it with a register, or split the signal so that no element depends on itself.
   * - E4123
     - A clocked process of another shape
     - ``if rising_edge(clk) then ... end if;`` with an optional asynchronous reset branch before it, nothing
       else in the process, clock and reset in the sensitivity list.
   * - E4124
     - A signal element with two drivers, a guarded signal, a resolution function of the design
     - Drive each element from one process.
   * - E4125
     - A clock made inside the design (gated, divided)
     - Make the clock in the testbench, or use a clock enable.
   * - E4126
     - A construct yavs does not follow far enough to show the process is logic
     - Limitation. Write it more simply (the label says how where it can), or use ``--cycle``.
   * - E4127
     - A process outside the region reads the history of a signal this process drives
     - Let the reader keep its own copy, or use an edge test.
   * - E4128
     - A ``force`` or ``release`` the design can run changes a signal this process drives (one that names a
       signal parameter, or any under ``--cycle-exact-meta``, keeps every process out); forces in units the
       design does not instantiate, and in subprograms nothing calls, do not count
     - Limitation. Drive the signal from the testbench instead, or use ``--cycle``.
   * - E4129
     - Logic feeding a process outside the region whose result depends on delta cycles (the second marker
       names it)
     - Fix that reader so that it can join too (complete sensitivity list, no latch), or make it clocked.
   * - E4130
     - Logic between clocks whose edges come in different delta cycles
     - Clock both registers from one clock, or make the second clock a plain copy (``clk_b <= clk_a;``).
   * - E4131
     - Logic that computes a clock from what the region drives
     - Make the clock in the testbench, or use a clock enable.
   * - E4132
     - A register reset by another register that drives a clock or reset
     - Use a synchronous reset, or a reset that does not come from a register.
   * - E4133
     - A memory written in its reset branch, at fixed and run-time indices both, or read outside the region
     - Limitation. Write it at run-time indices in the clock-edge branch and read it inside the design.
   * - E4134
     - A signal driven partly by a process outside the region (the second marker names it)
     - Limitation. Drive all of it from logic that joins, or split it into two signals.
   * - E4135
     - A driven signal that also gets a value from a port connection or a constant
     - Limitation. Drive the signal from one process only.
   * - E4136
     - An assert of the top-level unit reading a signal the region does not drive alone (``--cycle-report``
       only: the top-level unit is exempt)
     - Nothing to fix.
   * - E4137
     - With ``--cycle-exact-meta``: a metavalue the testbench reads
     - Run without ``--cycle-exact-meta``, or assign ``'0'`` or ``'1'``.
   * - E4138
     - An external name
     - Limitation. Pass the signal through a port.
   * - E4139
     - A loop whose bounds are not known during elaboration, a very large loop, ``exit`` or ``next`` of an outer
       loop
     - Use bounds from constants or generics, split a large loop, jump only out of the innermost loop.
   * - E9999
     - An internal error in yavs: a reason its own earlier checks should rule out
     - A bug in yavs: please report it with the design; ``--cycle`` keeps the process in the event simulation.

``yavs --explain E4119`` prints the whole explanation of one code; :doc:`error-codes` lists them all.

.. _cycle-cache:

Start-up cost and the cache
---------------------------

The first run of a design selects and analyses the region, and Cranelift compiles its functions. The time that takes
grows with the size of the region, not with the length of the simulation: for a short test of a large design it can be
longer than the time the faster simulation saves.

Both are kept in the compilation cache of the work library (``yavs-cache/cycle`` next to the library,
:doc:`on-disk`): the analysis (which processes run cycle-based, and their code before it is compiled) and the
compiled machine code, and the region as it was built from them. A later run of the same design takes the region from
the cache and links the code: its start-up is close to or below that of a run without ``--cycle`` (half of it on a large
accelerator whose region is most of the design). ``--cycle-strict`` and ``--cycle-report`` take the analysis and
build the region from it. In a test suite the test cases of one design therefore analyse
and compile the region once: a top-level generic counts only when the elaboration or the analysis reads its value, so
a test runner's configuration string, which only the testbench reads while it runs, does not keep two test cases
apart. An edited file, a generic that is read with another value, other options or ``YAVS_`` environment variables,
or another yavs build make new entries. A text file the elaboration reads is part of the key with its contents (a
program loaded into a memory at elaboration: another program makes new entries). A design whose elaboration writes a
file, reads another kind of file, an environment variable, the clock or foreign code keeps no analysis in the cache
(its functions are still cached one by one), and a design keeps at most
16 analyses: a testbench whose every test case reads generics of its own (a case name) shares only the functions. Test
cases whose analysis and region come out the same anyway keep one copy of them. The size of the store is
limited (16384 entries, 2 GiB, ``YAVS_CYCLE_CACHE_MIB`` sets the bytes, :doc:`environment`; a region of more
functions than that keeps its code as a whole only), and
``yavs cache clean --work=NAME:PATH`` empties it. ``--no-cache`` makes a run neither read nor write it, so such a run
analyses and compiles every time.

A run that compiles compiles the region's functions on as many threads as ``--threads`` gives (one by default,
``auto`` up to 8, at most the CPUs the run may use; under a test runner's token server the threads past the first
only on the tokens it lends, :doc:`threads`). The machine code, and so the result and what goes into the cache, is
the same on any number of threads; only the start-up of a run that finds nothing in the cache is shorter. Under
VUnit the token server of ``vunit-yavs`` is on by default and every test runs with ``--threads=auto``, so a test
compiles on the CPUs no other test uses, not more than the machine has (``--yavs-no-jobserver`` turns it off,
:doc:`vunit-yavs`).

When several tests start at once on a cold cache, each of them compiles the region. Run one test of the suite first, or
keep ``yavs-cache`` between CI runs (:doc:`ci`), so that the others find the code.

.. _cycle-troubleshooting:

Troubleshooting
---------------

**The results differ from the normal mode.** Go through the list:

1. Does the end of the run say that metavalues were read as ``0``? Then ``'U'``, ``'X'`` or an uninitialised
   register mattered. Fix the design or the test, check it in the normal mode, or run with ``--cycle-exact-meta``
   when the testbench checks for an ``'X'`` or a ``'Z'`` from the design.
2. Does a message of the design come at another delta, once instead of twice, or not at all? A combinational
   process reports on settled values only, and an assert that needs a ``'U'`` to fire does not fire on two-state
   values (:ref:`cycle-reports`). With ``--no-asserts``, RTL code that used to report, fail, or end the test does
   not do it.
3. Does the testbench read a value a delta after the edge, or depend on delta numbers? Sample later. Does the
   design copy a clock by an assignment and read registers of the source on the copy? See
   :ref:`cycle-clock-copies`.
4. Does the testbench read a signal that has no initial value at time 0? It reads ``'U'`` at delta 0 in both modes
   and ``0`` from the next delta on in cycle mode.

If none of these explains it, it is a bug in cycle mode: a wrong result is never an accepted loss. Report it with
the smallest design that shows it.

**A test hangs, or ends at another time.** If the summary line counts inputs driven by nothing, ``--cycle-report``
names them; a test that relies on their metavalue (see two-state values above) needs them driven in the testbench. If the end of the test came from RTL code (an ``assert`` or ``report`` of
severity failure in the design) and the run has ``--no-asserts``, the run goes on to the testbench's own end or to
``--stop-time``. Give the test a ``--stop-time``, or move the check into the testbench. If it is a testbench that waits
for a signal in the same delta as a register, see delta cycles above. ``--cycle-report`` shows whether the process
that raises the message joined.

**A design is slower, or not faster.** Read the first line of the report:

* Few processes joined: the rows of the report (or the errors of ``--cycle-strict``) and the ``boundary`` line say
  what keeps the rest out. A clocked checker of the top-level unit that holds an ``assert`` stays in the event
  simulation by design, and so does anything whose history the testbench reads.
* The testbench is most of the run: ``--profile`` shows where the time goes (:doc:`running-faster`). Cycle mode does
  not touch that part.
* The test is short: see the start-up cost above, and keep the cache between runs.

See also
--------

* :doc:`cli-reference`: ``--cycle``, ``--cycle-strict``, ``--cycle-report`` and ``--cycle-exact-meta``.
* :doc:`vunit-yavs`: the ``yavs.cycle`` sim option.
* :doc:`error-codes`: the codes E4101 to E4139.
* :doc:`running-faster`: the other ways to a faster run.
* :doc:`environment`: the ``YAVS_CYCLE_*`` variables for developers.
