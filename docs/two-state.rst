Two-state values
================

``--two-state`` makes the design under test compute with ``0`` and ``1`` only. It is a flag of its own: it
changes what a run computes with, not how it is scheduled, so it works with and without ``--cycle``. This page says
what it changes and what you give up, and how to use it to find out where a difference from the normal mode comes
from.

.. warning::

   ``--two-state`` is **experimental**, like cycle mode: its options, its error codes and what it gives up may change in
   a later release without a deprecation period.

.. contents:: On this page
   :local:
   :depth: 1

Two ways to use it
------------------

.. code-block:: bash

    yavs run --cycle --two-state --work=work:build/work tb_counter    # fast
    yavs run --two-state --work=work:build/work tb_counter            # diagnostic

* ``--cycle --two-state`` is the fast combination. The region computes two-state code, no process stays out of it for a
  metavalue, and its start-up does not follow the ``'U'`` start values (:ref:`cycle-meta`).
* ``--two-state`` alone, in the normal mode, is a diagnostic mode. It changes the values only, so it tells apart a
  difference that comes from metavalues from one that comes from the scheduling (:ref:`two-state-split`).
* Plain ``--cycle`` does not use two-state values: it computes with the normal mode's nine values
  (:ref:`cycle-meta`).

.. note::

   **Changed after 0.21.** In 0.21 plain ``--cycle`` was two-state. Now it computes with the nine values, and the
   option of 0.21 that kept metavalues is gone. To keep the results of 0.21, add ``--two-state`` (vunit-yavs: ``yavs.two_state``) where you
   gave plain ``--cycle``.

In the normal mode ``--two-state`` aims to run as fast as the default; today it is slower. Most unit tests run within a
few percent of the default, some up to about half again as long, and large designs such as processor cores take up to
about a tenth longer. Bringing it to parity is planned.

.. _two-state-normal:

What it does
------------

Every process of the two-state scope, and every subprogram declared in an architecture of it, is two-state. The scope
is the design under test yavs finds, and with ``--cycle`` without PATH that is the cycle scope too (:ref:`cycle-scope`), so the testbench and its
verification components (bus models and other processes that wait for a time) keep the nine values of ``std_ulogic``.
``--two-state=PATH`` (repeatable) names another scope, ``--two-state=TOP`` (the top-level unit's name) the whole design:

.. code-block:: bash

    yavs run --two-state=tb.dut --work=work:build/work tb       # one instance and what is below it
    yavs run --two-state=tb --work=work:build/work tb           # the whole design, testbench included

Without PATH one line on standard error names the design under test, and ``--cycle --cycle-report`` starts with the
scopes. With ``--cycle`` the region is two-state when the cycle scope lies inside the two-state scope; a cycle scope
partly inside it is an error (E2123).

* A value it reads from outside (a signal, a port, a generic, a constant of a package or an architecture, the result of
  a package's subprogram, an ``out`` argument a package procedure such as ``read`` gives back) counts ``'1'`` and
  ``'H'`` as ``'1'`` and every other value as ``'0'``. A port the testbench drives with ``'X'`` reads as ``'0'``. The
  subprograms of packages themselves keep the nine values: their callers map what they give back.
* A metavalue literal it assigns (``'U'``, ``'X'``, ``'Z'``, ``'W'``, ``'-'``) takes the ``--init-x`` value, ``'0'``
  without the option. ``'L'`` is ``'0'`` and ``'H'`` is ``'1'``. So does the literal of a constant, a record or an
  array of records too, wherever a part of it is read.
* A metavalue literal it compares with keeps its meaning: a ``0`` or ``1`` never equals ``'U'``, ``'X'``, ``'Z'`` or
  ``'W'``. So ``x = 'X'`` is false, ``x /= 'U'`` is true (an ``assert clk /= 'U'`` "is it connected" check holds),
  ``x ?= 'X'`` is ``'0'``, and a ``case`` choice ``when "0X"`` is never taken. A ``'-'`` in ``std_match``, ``?=``,
  ``?/=`` or a ``case?`` choice matches anything. The region does the same.
* A ``'Z'`` literal or constant driven onto a resolved signal (``std_logic``, ``std_logic_vector``) stays ``'Z'``, so
  tri-state buses and the event signals of verification components in the scope work. Reading the bus back gives
  ``'0'`` while nothing drives it.
* A signal or process variable without an initial value starts at the ``--init-unset`` value, ``'0'`` without the
  option. A variable of a subprogram starts at ``'0'``.

``--init-x`` and ``--init-unset`` are options of their own (:doc:`start-values`): run the suite with ``0``, ``1`` and
``random`` to see whether a result depends on them.

What you give up
----------------

With or without ``--cycle``:

* A bug that shows as ``'X'`` or ``'U'`` in the normal mode (a register without reset, an unconnected input, a conflict
  on a bus) cannot show in the scope. A testbench that expects ``'X'`` or ``'Z'`` from the design sees ``'0'``.
* A ``'Z'`` read in the scope is a ``'0'``. A verification component or model in the scope that checks a line for
  ``'Z'`` or models its bus with ``'Z'`` and ``'X'`` (an MDIO, I2C, 1-wire or QSPI flash model, a bidirectional bus
  monitor) reads the released line as ``'0'``, so it can report protocol errors that are not there or answer with the
  wrong data. Such a component waits for time, so the design under test leaves it out and it keeps the nine values;
  check the ``design under test`` line when a model is written as RTL only, and when you give ``--two-state=PATH``.
* A process of the design under test that assigns a ``'Z'`` literal, such as the tri-state driver of a pad model
  (``pad <= o when oe = '1' else 'Z';``), is left out of the scope found too, so the line keeps its ``'Z'`` and what
  the testbench drives on it; the rest of the design stays two-state. A ``--two-state=PATH`` keeps such a process in
  when the path covers it.
* Only a ``'Z'`` written in the driven value itself stays ``'Z'``: one kept in a variable first is a ``'0'`` when
  driven. A pull-up or pull-down literal (``'H'``, ``'L'``) drives a strong ``'1'`` or ``'0'``, so pull-ups and
  open-drain buses in the scope break: a driver pulling the line low conflicts with the pull-up, and the
  line reads as ``'0'``.
* A clock copied in the scope (``clk2 <= clk``) from a clock that starts at ``'U'`` starts at ``'0'``, so its
  first rise is a ``rising_edge`` the normal mode does not see.

.. _cycle-two-state:

With ``--cycle``
~~~~~~~~~~~~~~~~

The region computes with ``0`` and ``1`` too, and these hold for it as well:

* A metavalue that reaches the region from outside (``'X'``, ``'U'``, ``'W'``, ``'-'``, ``'Z'`` and the like) reads as
  ``0``; ``'1'`` and ``'H'`` read as ``1``, ``'0'`` and ``'L'`` as ``0``. yavs counts the elements read this way and
  prints the count at the end of the run. It counts reads, not signals: an element read as a metavalue at two wakes of
  the region counts twice, and a metavalue that comes and goes between two reads is not counted.
* A change between two values with the same two-state value (``'1'`` to ``'H'``, ``'U'`` to ``'X'``) is no change in
  the region: a combinational process of the region does not run for it, and makes no ``report`` or ``assert`` message
  where the normal mode makes one.
* A cycle-based process that copies a line another driver pulls up (a released ``'H'``) to a signal the testbench reads
  gives the testbench ``'1'`` where the normal mode gives ``'H'``. ``to_x01`` makes ``'1'`` of both.
* The region gives back only ``'0'`` and ``'1'``: a process that assigns a metavalue literal (``(others => 'X')`` on
  invalid data, say) gives the ``--init-x`` value there, ``'0'`` without the option, and so does one that copies a
  tri-state line some process outside sets to ``'Z'``. A bus model that checks read data against an expected ``'X'`` fails.
* A signal the region reads that nothing drives keeps its ``'U'`` for the whole run in the normal mode, and reads as
  ``0`` in the region. A test that worked only because of that ``'U'`` (an undriven ``valid``) can now behave
  differently, or never end. The first summary line then adds ``, N inputs driven by nothing read as 0``, and
  ``--cycle-report`` names them. An unused field of a record port is counted too; it changes nothing.
* The asserts and reports of the region see these values. An assert that fires in the normal mode only because a value
  is still ``'U'`` (a register before its reset, ``unsigned(addr) < DEPTH`` on an uninitialised address) does not fire,
  and one that holds only because of a metavalue may fire. No cycle-based process gives the library warnings of
  ``numeric_std``.
* A failed check (overflow, range, index) says what failed but not the values, which saves code in every check.
  Without ``--two-state`` the message is the normal mode's (:ref:`cycle-checks`).
* ``--wave`` records the region's two-state values: a signal without an initial value shows ``0`` where the normal
  mode shows ``'U'`` (:ref:`cycle-waves`).

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

``--cycle --two-state``: ``ticks`` is ``0000`` from the start, the ``'X'`` on ``a`` reads as ``0`` so the comparison is
true, and the library warning is not produced.

.. code-block:: text

    design under test: tb_metavalue.dut (the scope of --cycle and --two-state; an option's PATH chooses another)
    cycle mode: 2 of 4 processes run cycle-based, 2 in the event kernel (see --cycle-report)
    ** Note: 0fs+0: at 0 ns:  ticks=0000
        Process tb_metavalue.main
    ** Note: 1000000fs+0: at 1 ns:  ticks=0000
        Process tb_metavalue.main
    ** Note: 17000000fs+0: at 17 ns: eq=1 with 'X' on a
        Process tb_metavalue.main
    cycle mode: 4 metavalue elements read as 0 (0 at initialisation, 4 at changes of the region's inputs)
    ** Note: 17000000fs+0: FINISH called with status 0

Plain ``--cycle`` on the same design is in :ref:`cycle-meta`.

.. _two-state-split:

Splitting a difference
----------------------

When a test gives another result under ``--cycle --two-state`` than in the normal mode, two more runs tell where it
comes from:

1. Run the normal mode with and without ``--two-state``. A difference there comes from the values: the design reads or
   makes a metavalue that the two-state values turn into ``0`` or ``1``.
2. Run ``--two-state`` with and without ``--cycle``. A difference there comes from the scheduling: the order and deltas
   of cycle mode (:ref:`cycle-deltas`, :ref:`copied clocks <cycle-clock-copies>`).

.. code-block:: text

    yavs run --work=work:build/work tb_counter
    yavs run --two-state --work=work:build/work tb_counter
    yavs run --cycle --two-state --work=work:build/work tb_counter

The vunit-yavs sim option ``yavs.two_state`` set to ``True`` adds ``--two-state``, set to a path or a list of paths
``--two-state=PATH`` for each, with or without ``yavs.cycle`` (:doc:`vunit-yavs`):

.. code-block:: python

    lib.set_sim_option("yavs.cycle", "on")
    lib.set_sim_option("yavs.two_state", True)

See also
--------

* :doc:`cycle-mode`: the scheduling, what joins, the report and the cache, and what plain ``--cycle`` does with
  metavalues.
* :doc:`start-values`: ``--init-unset`` and ``--init-x``.
* :doc:`running-faster`: ``--cycle --two-state`` as a speed option.
