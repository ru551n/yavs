Cycle mode
==========

Cycle mode runs the clocked and combinational RTL of a design as compiled code, evaluated once per clock edge, and
leaves the testbench in the normal simulation. It is faster on long runs of synchronous designs. This page is about
the scheduling: what runs as a region, what that changes in a run, how to see and steer which processes join, and the
cache. What the region computes with is chosen by ``--two-state`` (:doc:`two-state`); what plain ``--cycle`` does with
metavalues is :ref:`cycle-meta` below.

.. warning::

   Cycle mode is **experimental**. Nothing changes unless you ask for it with a cycle flag. Its options
   (``--cycle``, ``--cycle-strict``, ``--cycle-report``, ``--two-state``, the vunit-yavs sim options
   ``yavs.cycle`` and ``yavs.two_state``), its error codes, the processes it accepts and its results may change in a later release
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

The gain depends on how much of the design joins and how much of the run the testbench takes. yavs does not predict
it: run your test both ways and compare. The other ways to a faster run are on :doc:`running-faster`.

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
    cycle mode: 'U' start values followed on a second image: the images agreed after 16 clock edges
    ** Note: 3016000000fs+0: FINISH called with status 0

The testbench's output is the same. The ``cycle mode`` lines go to standard error:

* The first says how many processes joined. When no process joins, it says ``0 of N`` and the run is a normal run.
* The second, printed only when it happened, counts the metavalue elements (``'U'``, ``'X'`` and so on) the region read
  as ``0``: here the start values of signals without an initial value.
* The third says that the region followed those start values on a second set of values until the two agreed
  (:ref:`cycle-meta`). ``--two-state`` leaves that line out, and that work.

``yavs run`` has two cycle flags and a third that modifies them. ``--cycle`` and ``--cycle-strict`` are
alternatives; both together are a usage error (exit status 2).

.. code-block:: bash

    yavs run --cycle --work=work:build/work tb_counter                 # run; what does not fit stays out
    yavs run --cycle-strict --work=work:build/work tb_counter          # what does not fit is an error
    yavs run --cycle --cycle-report --work=work:build/work tb_counter  # say what would join; run nothing

``--cycle-report`` without one of the two is an error (E4105). Both take a scope, the part of the design whose
processes may join: ``--cycle=PATH`` names it; without PATH it is the whole design, and with ``--two-state`` the
design under test that yavs finds (:ref:`cycle-scope`).

``--two-state`` goes with either, or alone. With it the region computes on ``0`` and ``1`` only: more processes
join and the run is faster. Without it plain ``--cycle`` keeps the metavalues the testbench would see (:ref:`cycle-meta`).
The values come from ``--two-state`` alone, the same for ``--cycle`` and ``--cycle-strict``; the rest of this page is
about the scheduling, and ``--two-state`` is on :doc:`two-state`.

.. code-block:: bash

    yavs run --cycle --two-state --work=work:build/work tb_counter

.. note::

   **Changed after 0.21.** In 0.21 plain ``--cycle`` was two-state. Now plain ``--cycle`` and ``--cycle-strict`` keep
   the metavalues (processes that would hide a metavalue from the testbench stay in the event kernel), and the option
   of 0.21 that kept them is gone. To keep the results of 0.21, add ``--two-state`` (vunit-yavs: ``yavs.two_state``)
   where you gave plain ``--cycle``.

``--no-asserts`` is a flag of every run, not of cycle mode: it drops the design's own asserts, and is described with
the other speed options (:ref:`no-asserts`). ``--init-unset`` and ``--init-x`` are flags of every run too, for
checking a design's reset (:doc:`start-values`); with ``--cycle`` they give the region the start values and literal
values the normal mode gets.

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
* A test with ``yavs.cycle`` on that also asks for coverage (``--yavs-cov``, ``enable_coverage``), ``--yavs-debug`` or
  ``--yavs-interactive`` fails before yavs starts, with a message that names both options. So a coverage job needs
  ``yavs.cycle`` ``"off"``. ``--yavs-surfer`` works. A ``--wave-variables``, ``--cov`` or ``--vhpi`` in
  ``yavs.sim_flags`` is refused by yavs itself (E4102).
* ``yavs.cycle_scope`` (a path, or a list of paths) gives ``--cycle`` its PATH (:ref:`cycle-scope`); without
  ``yavs.cycle`` it is refused at the start of the test.
* To see the report of one test, set ``yavs.sim_flags`` to ``["--cycle-report"]`` for that test, with
  ``yavs.cycle`` ``"on"``. yavs prints the report and simulates nothing, so the test fails on purpose: use it for a
  look on your machine, with ``-v``, not in CI.
* ``yavs.no_asserts`` is the sim option of ``--no-asserts`` (:ref:`no-asserts`), and ``yavs.init_unset`` and
  ``yavs.init_x`` those of the start-value flags (:doc:`start-values`). ``yavs.two_state`` set to ``True`` adds
  ``--two-state``, set to a path or a list of paths ``--two-state=PATH`` for each (:doc:`two-state`).

.. code-block:: python

    # Only the design and its memory, cycle-based and two-state.
    lib.set_sim_option("yavs.cycle", "on")
    lib.set_sim_option("yavs.cycle_scope", ["tb.dut", "tb.mem"])
    lib.set_sim_option("yavs.two_state", ["tb.dut", "tb.mem"])

.. _cycle-scope:

The scope: which part of the design
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Only the processes of the *cycle scope* may join the region; every other process stays in the event simulation,
whatever it is. ``--two-state`` has a scope of its own (:doc:`two-state`). The defaults, when no PATH is given:

* ``--cycle`` (or ``--cycle-strict``) alone: the whole design. Every process that fits may join, those of the
  testbench and its verification components included, so the most processes run cycle-based; the processes of the
  top-level unit keep their own rules (one with an assert or report that cycle mode would evaluate only after the
  region settles stays in the event kernel, and under ``--cycle-strict`` none of them is an error).
* ``--two-state``, alone or with ``--cycle``: the *design under test*, which yavs finds without being told. With
  ``--cycle`` without PATH the cycle scope is the design under test too, so the region is two-state and the
  testbench's models and checkers keep the nine values.

A path names another scope:

.. code-block:: bash

    yavs run --cycle=tb.dut --work=work:build/work tb                  # one instance and what is below it
    yavs run --cycle=tb.dut --cycle=tb.mem --work=work:build/work tb   # two of them
    yavs run --cycle=tb.g --work=work:build/work tb                    # every iteration of generate g
    yavs run --cycle=tb.g(1) --work=work:build/work tb                 # one of them
    yavs run --cycle=tb --two-state=tb --work=work:build/work tb       # the whole design, testbench included

A path names an instance, a block, a generate or a labelled process from the top-level unit down, as ``--print-hierarchy`` and the
report write them, without case; everything at and below it is in the scope. One that names nothing is an error
that names a similar path (E2122). The processes of the top-level unit are in a named scope only when the path is the
top-level unit's name; there they follow the rules of every other process of the scope (under ``--cycle-strict`` an
RTL-shaped one that cannot join is an error). ``--no-asserts`` does not follow the scope: it drops the asserts and
reports below the top-level unit (:ref:`no-asserts`).

**The design under test.** A testbench often holds verification components next to the design: a bus model, a
memory model, a clock generator, a checker. yavs finds the design from what the processes do, never from names. A
process is of the testbench when it waits for a time, waits more than once, calls a procedure that waits, uses
files, text input and output, protected objects or access values, or makes a waveform with ``after`` from nothing or
from its own output without a clock (``clk <= not clk after 5 ns;``); a process that runs ``wait;`` once and every
other process are RTL, delayed assignments of logic included. From the top-level unit down, the design under test is
each instance of a level that holds RTL and whose processes, its own and those below it, are at most one in twenty
of the testbench kind. When an instance holds RTL and more of the testbench (a harness around the design and its
models), yavs looks inside it the same way. The scope is all the instances found; the top-level unit itself when
none is found and it holds RTL of its own, its testbench-kind processes left out (so a flat testbench's stimulus
keeps the nine values); and empty when the design has no RTL at all, which ``--cycle-strict --two-state`` without
a cycle PATH refuses (E4106). A process that assigns a ``'Z'`` literal (a pad's tri-state driver) is left out of the
scope found, so it keeps the nine values (:doc:`two-state`). A memory or FIFO the
testbench instantiates next to the design is found too, being RTL: that is usually what you want, and a path
leaves it out.

Without PATH, one line on standard error names it:

.. code-block:: text

    design under test: tb.dut, tb.mem (the scope of --cycle and --two-state; an option's PATH chooses another)

The report (:ref:`cycle-report`) starts with the scopes. A process that could join but is outside the cycle scope
is listed under ``outside the cycle scope``.

**The two scopes together.** They may differ, but the cycle region computes with one kind of values: two-state when
the cycle scope lies inside the ``--two-state`` scope, with the metavalues when it lies outside it (no
``--two-state``, or a ``--two-state`` scope elsewhere). Scopes that overlap in part are an error (E2123), such as
``--cycle=tb --two-state=tb.dut``.

.. note::

   **Changed in this release.** ``--two-state`` used to cover everything below the top-level unit; it now covers the
   design under test, and so does ``--cycle`` without PATH when ``--two-state`` is given. For the old reach of
   ``--two-state``, name the instances below the top. Plain ``--cycle`` without PATH still takes the whole design.
   ``--cycle-report`` no longer turns cycle mode on by itself.

.. _cycle-changes:

What you give up
----------------

Read this before you trust a green run. Processes in the event kernel behave exactly as in the normal mode. For the
cycle-based processes, this is the whole list of differences that come from the scheduling:

* delta cycles inside a clock edge are not visible, and a copied clock is the same clock as its source
  (:ref:`cycle-deltas`);
* their ``assert`` and ``report`` statements are evaluated, but on the values of the clock edge or of the settled time
  step, so a message can come at another delta or once instead of twice (:ref:`cycle-reports`), and the ``ieee``
  library warnings are not given;
* with ``--two-state``, a failed check does not print its values (:ref:`cycle-checks`);
* waveforms show the region's signals per time step, without deltas (:ref:`cycle-waves`);
* a delayed assignment outside the cycle scope stays in the event kernel, so a one-delta glitch of cycle-based logic
  feeding it can change that waveform. Compute the delayed value from registered signals, or check
  timing-sensitive waveforms in the normal mode;
* coverage, VHPI, activity counts, variable waveforms and the debugger are refused (:ref:`cycle-refused`).

What the region computes with is not on this list, because it changes with ``--two-state`` and not with the
scheduling: with ``--two-state`` metavalues read as ``0`` and ``1`` (:doc:`two-state`); without it plain ``--cycle`` keeps
the metavalues the testbench would see, with the limits in :ref:`cycle-meta`.

Two-state values
~~~~~~~~~~~~~~~~

Moved to :doc:`two-state`.

If a test differs between the normal mode and ``--cycle`` for any other reason, it is a bug in cycle mode: please
`report it <https://github.com/ru551n/yavs/issues>`__.

.. _cycle-reports:

Asserts and reports
~~~~~~~~~~~~~~~~~~~

Cycle-based processes evaluate their ``assert`` and ``report`` statements by default, with the normal mode's message
text, severity, time and process name, and the same effect on the run: an ``error`` counts for the exit status, a
``failure`` (or whatever ``--exit-severity`` names) ends the run at the time the normal mode ends it. A design that
ends its own test from RTL code ends it in cycle mode too. ``--no-asserts`` is the way to drop them for speed
(:ref:`no-asserts`).

* A clocked process evaluates them at its clock edge, on the values before the edge, as the normal mode does.
* A combinational process evaluates them once its inputs are settled, and only in a wake where an element it waits on
  (its sensitivity list, or what it reads for ``process (all)`` and a concurrent ``assert``) changed since its last
  run: the event that runs it in the normal mode. The region computes its logic in dependency order, so the values an
  assert sees are the settled values of that time step, never a value the region passes through.
* No cycle-based process gives the warnings of ``numeric_std`` and the other ``ieee`` and ``std`` packages
  (metavalue and truncation warnings).

What can differ:

* **Delta and order.** A message of the region carries the delta of the region's wake; in the normal mode a message
  of combinational logic comes some deltas later. Within one run of the region the clocked processes' messages come
  first, then the combinational ones', each in elaboration order. Against the messages of processes in the event
  kernel they come in the order of the wakes, so within one time step a message of the region may come before or after
  the one the normal mode prints before or after it.
* **Intermediate values.** In the normal mode a combinational process runs again in every delta where one of its
  inputs changes, so it can report a value its inputs hold for one delta only, or report twice in one time step. In
  cycle mode it runs once per wake on settled values: such a message is made once, with the settled values, or not at
  all when the settled values satisfy the assert.
* **Values.** With ``--two-state`` an assert sees the region's values (:ref:`cycle-two-state`): one that fires only
  because a value is still ``'U'`` does not fire. Without it a process of the cycle scope with an assert or
  report that reads a signal the region does not drive alone stays in the event kernel (E4140), so it sees the
  metavalues the rest of the design gives it.
* ``--assert-summary`` and ``--exit-severity`` count the messages of the region as those of the normal mode.
* A design's own message from a function called with constant arguments (``log2(DEPTH)``, a check of a register
  map) appears once per process at 0 fs, instead of each time the statement runs, and also when the run never takes the
  branch that holds the call. A function that reports with severity error or failure keeps its process in the event
  kernel.
* A known gap: when every process around a constant joins, an ``ieee`` warning or an overflow in the constant's
  initial value (a truncating ``to_unsigned``) is not produced, and the overflow does not stop the run.

An assert or report cycle mode cannot evaluate keeps its process in the event kernel (E4140; an error under
``--cycle-strict``). It takes a message made of string literals, ``&``, ``'image`` and
``to_string`` of integers, enumerations, ``std_ulogic`` and ``boolean`` values, and ``to_string``, ``to_hstring`` and
``to_ostring`` of vectors, with a severity known during elaboration. It does not take one in the asynchronous reset
branch of a clocked process, or in a combinational process whose sensitivity list names a signal the process does not
read. Write the message in that form, move the check out of the reset branch, or use ``--no-asserts``.

The testbench's own checks are outside the cycle scope by default (:ref:`cycle-scope`) and run in the event kernel as
in the normal mode. With the top-level unit in the scope (``--cycle=TOP``) its processes follow the rules above, as
every other process of the scope. VUnit ``check_*`` calls in the testbench process run as before.

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

``--cycle --two-state`` (plain ``--cycle`` gives the same messages, and one more ``cycle mode`` line about the second
image):

.. code-block:: text

    design under test: tb_delta (the scope of --cycle and --two-state; an option's PATH chooses another)
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
status, line, time and process, but without the values. A division by zero and a quotient too large for its type give
the same message. Without ``--two-state`` the message is the normal mode's, with the values; ``--two-state`` leaves
them out, which saves code in every check. ``overflow.vhd`` counts an integer past its largest value, with
``--cycle --two-state``:

.. code-block:: text

    design under test: tb_overflow.dut (the scope of --cycle and --two-state; an option's PATH chooses another)
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

.. _cycle-waves:

Waveforms
~~~~~~~~~

``--wave`` works with ``--cycle`` and ``--cycle-strict``, and so do ``--wave-include``, ``--wave-exclude``,
``--wave-max-array`` and ``--surfer`` (:doc:`waveforms`). The file has the names of a normal-mode file and its values
per time step, as far as the region allows:

* Signals the event kernel holds (those of processes outside the region, the inputs of the region and the outputs the
  testbench reads) are recorded as in the normal mode.
* A signal only the region reads is exported from the region after each clock edge and after each settle of an input
  change, and recorded from there. A value changes in the time step where it changes in the normal mode, with its last
  value of that time step. The delta cycles of the normal mode and any glitch between them are not there. The file keeps
  one value per time step in both modes, so this shows only for a signal that changes and changes back within a time
  step: the normal mode's file may write its value again at that time, this one writes nothing.
* The values are the region's: with ``--two-state`` ``0`` and ``1``, so a signal without an initial value shows ``0``
  where the normal mode shows ``'U'`` (:ref:`cycle-two-state`). Without it the second set of values gives ``'U'`` and
  ``'X'`` where the testbench would read them, while it runs (:ref:`cycle-meta`).
* Memories are recorded like other arrays (up to ``--wave-max-array`` elements), the words written at each edge
  exported. One kind cannot be exported and is left out of the file, with warning W0802 naming it: a signal whose
  elements are integers or enumerations (other than ``std_ulogic``, ``std_logic``, ``bit`` and ``boolean``), of more
  than 64 elements, that a clocked cycle-based process writes at an index computed at run time. The whole signal is left
  out (a record holding such a field too, and a port connected to it); every other signal is recorded as usual:

  .. code-block:: vhdl

      type iram_t is array (0 to 127) of integer;
      signal iram : iram_t;
      ...
      if rising_edge(clk) then
        iram(cnt mod 128) <= cnt * 7;   -- iram is left out of the waveform under --cycle
      end if;

  A memory of ``std_logic`` vectors, an integer array of at most 64 elements, one written only at fixed indices or as
  a whole, and one written by a process in the event kernel are recorded.
* Variables are not recorded: ``--wave-variables`` is refused (E4102).
* Without ``--wave`` nothing changes. With it, recording costs what it costs in the normal mode, and the region also
  exports each recorded signal at every time step where it changes. It exports only the signals the selection keeps, so
  ``--wave-include`` of a few signals avoids most of that second part. The region of each selection is analysed and
  compiled on its first run and taken from the cache after that.

.. _cycle-refused:

Coverage, VHPI, activity counts and the debugger are refused
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The region keeps its signals in an image of its own: it exports what a waveform records, and nothing else can observe or
change them there. ``--cycle`` and ``--cycle-strict`` stop with E4102, before anything runs, when combined with
``--wave-variables``, ``--cov``, ``--vhpi``, ``--activity`` or a debug session (``--dap``, ``--interactive``, ``--do``):

.. code-block:: text

    error[E4102]: `--cycle` cannot be used with `--activity`
     |
     = note: cycle mode keeps the signals inside its region in its own image, which this option would observe or change
     = help: run without `--cycle`

    error: aborting due to 1 previous error

Use the normal mode for those. These stay allowed: ``--wave`` and ``--surfer``, ``--profile`` (the region's time shows
as the kernel's, and only the processes outside it are listed), ``--assert-summary``, ``--shuffle``, ``--hang-report``,
``--time-report`` and ``--threads``. ``--tier`` does not apply to the region, which is always compiled.
``--cycle-report`` does not simulate, so ``--wave`` is fine with it.

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
  (:ref:`cycle-meta`, :ref:`cycle-two-state`). The summary line names inputs driven by nothing.
* The testbench or a bus model that expects an ``'X'`` or ``'Z'`` from the design reads ``0`` with ``--two-state``, and
  a process that would hide one stays in the event kernel without it (:ref:`cycle-meta`).
* A message of the design comes at another delta, once instead of twice, or not at all (:ref:`cycle-reports`).
* The testbench samples a delta after an edge, depends on delta numbers, or the design reads a source register on a
  copied clock (:ref:`cycle-deltas`).
* The testbench reads a signal without an initial value at time 0.

Repeat the comparison after large changes to the design or the testbench.

``--cycle-strict``
~~~~~~~~~~~~~~~~~~

Use it for a design that should join as a whole, for example in CI, so that a change that moves a process out of the
region fails the job. It changes the selection only: each process of the cycle scope that cannot join is an error instead of
staying out, and the run stops before simulating, with exit status 2. What the region computes and what it evaluates
is the same as with ``--cycle``, so it does not make results more trustworthy; processes outside the cycle scope and
behavioural processes (:ref:`cycle-selection`) are never an error. The errors are shown in :ref:`cycle-stays-out`.

.. _cycle-meta:

Metavalues without ``--two-state``
----------------------------------

The aim is that plain ``--cycle`` computes with the normal mode's nine values and differs from it only in the
scheduling. A region with those values is a later step and is not in this version. Until it exists, plain ``--cycle``
keeps the metavalues the testbench would see with the mechanism and the limits below, and it is replaced when that
region lands. ``--two-state`` (:doc:`two-state`) drops all of this and runs faster.

Use plain ``--cycle`` when the testbench checks for ``'X'``, ``'U'`` or ``'Z'`` from the design, or checks reset state,
and to see the values of a failed check (:ref:`cycle-checks`). It is not the normal mode: it is closer, and only for a
while.

* A process that assigns a metavalue literal (``(others => 'X')``) to a signal the testbench reads, or passes on a
  ``'Z'`` from outside, stays in the event kernel (E4137), so the testbench sees the normal mode's value. So does a
  process with a comparison with a metavalue literal (E4101).
* Registers and signals without an initial value show the testbench ``'U'`` while their value depends on that start
  value, for up to 1024 clock edges. To know which bits depend on it, yavs runs the region a second time with those
  start values at ``1`` and compares the two. A bit fed only by ``'X'``, ``'Z'``, ``'W'`` or ``'-'`` shows ``'X'``.
  The event from ``'U'`` to ``0`` at time 0 is gone, and a bit that does not depend on the start value is ``0`` or
  ``1`` (``q and not q`` gives ``0`` where the normal mode gives ``'U'``).
* Failed checks give their values, with the normal mode's message.
* The asserts and reports of the region show ``'U'`` or ``'X'`` where the two runs differ, as the outputs do. A process
  of the cycle scope with an assert or report that reads a signal the region does not drive alone stays in the
  event kernel (E4140).
* The price is speed: more processes stay out, and the region does more work at first.
* ``--init-unset`` gives the start values a value from time 0, so no ``'U'`` start value is left to follow
  (:doc:`start-values`).

Known differences from the normal mode, with plain ``--cycle``:

* The start values are followed for at most 1024 clock edges of any clock. After that an ``'X'`` or ``'U'`` that comes
  in from outside the region arrives at the testbench as ``'0'``.
* A choice (``if``, ``case``) on a ``'U'`` can take the branch for ``0``. A choice on a variable can show ``'U'`` where
  the normal mode has a value.
* A clocked process can keep a register's old value where only the run on start values of ``1`` changed it.
* ``numeric_std`` arithmetic on a metavalue gives ``'U'`` or ``'X'`` only in the bits that depend on it, not all
  ``'X'``.
* A ``'U'`` that comes back to the region through a process outside reads as ``0``.

With ``metavalue.vhd`` of :ref:`cycle-two-state`, plain ``--cycle`` keeps ``ticks`` at ``'U'`` as the normal mode does.
``eq`` is ``1``, as with ``--two-state`` (the register kept its value, as in the third point), where the normal mode
gives ``0``:

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

The rules of the second run, and why it differs where it does, are in the developer guide of the crate
(`crates/yavs-cycle/AGENTS.md <https://github.com/ru551n/yavs/blob/main/crates/yavs-cycle/AGENTS.md>`__).

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

    yavs run --cycle --cycle-report --work=work:build/work tb_left_out

.. code-block:: text

    cycle mode: 1 of 7 processes run cycle-based, on the rising edge of tb_left_out.clk
      cycle scope: the whole design (the top-level unit under its own rules)
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

Read it from the top:

* The first line: how many processes join, and on which clock edges they run.
* The processes that stay out, grouped by reason, the most common first. Each row has the process, where it starts,
  and the reason at the line that causes it. A concurrent assignment shows as ``name <= ...``. The testbench's clock
  (``clk <= not clk after ...``) is on almost every list and is expected. *Behavioural* processes are testbench
  code by nature (:ref:`cycle-selection`).
* Two fixed lines: no ``ieee`` library warnings from the region, and how asserts and reports are evaluated.
* ``subset``: how many processes fit the rules on their own. A few may still stay out because of the design around
  them. A *loop group* is a set of processes that read each other's outputs without forming a combinational loop;
  they cost more per clock.
* ``boundary``: the signals between the region and the event kernel. ``exported`` are the signals the region drives
  for processes outside it, ``inputs`` those it reads from them, and ``wake the region`` the inputs that make it run
  between clock edges. Every such signal is converted and compared, so large numbers here cost speed. A ``wakes:``
  line may follow.
* The cycle-based processes, each ``clocked`` or ``combinational``, and the inputs nothing drives, if any.

With ``--no-asserts`` the report also lists the ``assert`` and ``report`` statements that are dropped, and the run
starts with a note that counts them (:ref:`no-asserts`).

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
* **Every process outside the cycle scope** (by default the testbench and its verification components,
  :ref:`cycle-scope`) stays in the event kernel and is never an error; one that could join is listed under
  ``outside the cycle scope``.

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
     - Uses a construct yavs does not implement yet (the message names it), such as a ``while`` loop; with
       without ``--two-state`` also a comparison with a metavalue literal
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
       any force without ``--two-state``, keeps every process out)
     - Limitation. Force from the testbench instead.
   * - E4129
     - Feeds a process outside the region that depends on delta cycles (the message names it). A delayed
       assignment (``after``, ``transport``, ``reject``) in the cycle scope is such a process: a one-delta
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
     - Retired: processes outside the cycle scope never join, and those inside it follow one set of rules
     - Nothing to fix.
   * - E4137
     - Without ``--two-state``: would hide a metavalue from the testbench
     - Assign ``'0'`` or ``'1'``, or run with ``--two-state``.
   * - E4138
     - Uses an external name
     - Limitation. Pass the signal through a port.
   * - E4139
     - Has a loop with unknown bounds, a very large loop, or ``exit`` or ``next`` of an outer loop
     - Use bounds from constants or generics; jump only out of the innermost loop.
   * - E4140
     - Has an assert or report cycle mode cannot evaluate (:ref:`cycle-reports`)
     - Write the message in a form it takes, move the check to the testbench, or use ``--no-asserts``.
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
  Large numbers in the ``boundary`` line mean the split between region and event kernel is expensive. The scope
  lines at the start of the report say what may join at all (:ref:`cycle-scope`).
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
* Keep coverage and debug jobs in the normal mode: with ``yavs.cycle`` on, they fail every test.
* Use ``--cycle-strict`` (``"strict"`` in VUnit) on designs that should join as a whole, so that a change that moves
  a process out fails the job.
* Look in the test output for the ``cycle mode:`` lines: a metavalue count that changes is worth a look.
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
model that expects ``'X'`` or ``'Z'`` from the design fails by design with ``--two-state`` (:ref:`cycle-two-state`):
run that test without it.

**A test hangs, or ends at another time.** If the summary line counts inputs driven by nothing, ``--cycle-report``
names them: drive them in the testbench. If the testbench waits a delta after an edge, see :ref:`cycle-deltas`. If the
run has ``--no-asserts``, RTL code that used to end the test with an assert no longer does: give the run a
``--stop-time``, or move the check into the testbench.

**A test fails in the normal mode and passes with** ``--cycle``. With ``--two-state`` a metavalue was read as ``0``, or
an assert that needs a ``'U'`` to fire does not fire (:ref:`cycle-two-state`). Trust the normal mode, and split the
difference (:ref:`two-state-split`).

**Nothing joins.** The summary says ``0 of N``. Run ``--cycle-report`` and look at the largest group. A design
whose RTL uses ``after`` delays or ``'Z'`` everywhere does not fit.

**An option is refused with E4102.** Coverage, VHPI, activity counts, variable waveforms and the debugger need the
normal mode (:ref:`cycle-refused`).

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
   * - ``--cycle[=PATH]``
     - Runs what fits of the cycle scope cycle-based and the rest in the event kernel. Selection never fails the
       run. PATH (repeatable) names the scope, the design under test without it (:ref:`cycle-scope`); one that
       names nothing is E2122.
   * - ``--cycle-strict[=PATH]``
     - As ``--cycle``, but a process of the cycle scope that is not behavioural and cannot join is an
       error; the run stops before simulating, exit status 2.
   * - ``--cycle-report[=FILE]``
     - With ``--cycle`` or ``--cycle-strict``: prints the report (or writes it to FILE, printing nothing), compiles
       and simulates nothing, exit status 0 (``--cycle-strict``'s errors first). Alone it is E4105.
       An unwritable FILE is E4103.
   * - ``--two-state[=PATH]``
     - With one of the above, or alone: its scope (the design under test without PATH) computes with ``0`` and
       ``1`` (:doc:`two-state`). A cycle scope partly inside it is E2123.

* ``--cycle`` and ``--cycle-strict`` exclude each other (exit status 2). ``yavs analyze`` and ``yavs elab`` take none of them.
* ``--no-asserts`` is a flag of every run (:ref:`no-asserts`, :doc:`cli-reference`).
* vunit-yavs: the sim options ``yavs.cycle`` (``"off"``, ``"on"``, ``"strict"``), ``yavs.cycle_scope`` and ``yavs.no_asserts``,
  :doc:`vunit-yavs`.
* Environment: ``YAVS_CYCLE_CACHE_MIB``, the limit of the cycle cache in MiB (default 2048), :doc:`environment`.
  The other ``YAVS_CYCLE_*`` variables are for developers of yavs.
* Codes: E4101 to E4104 and E4110 to E4140 in :doc:`error-codes`, and ``yavs --explain CODE``.

See also
--------

* :doc:`two-state`: the values the region computes with.
* :doc:`start-values`: ``--init-unset`` and ``--init-x``, for checking a design's reset.
* :doc:`running-faster`: the other ways to a faster run, and ``--no-asserts``.
* :doc:`cli-reference`: the cycle flags.
* :doc:`vunit-yavs`: the ``yavs.cycle`` sim option.
* :doc:`waveforms`: ``--wave`` and ``--surfer``.
* :doc:`threads`: threads and the token server, which the region's compilation also uses.
