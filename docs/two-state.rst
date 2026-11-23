Values in cycle mode
====================

This page is about the values of a cycle mode run: what the cycle-based processes compute with, and what the
testbench and the messages see of it. How the run is scheduled is on :doc:`cycle-mode`.

.. warning::

   This page describes the value handling of the development version, and it is an **interim**. A plain ``--cycle``
   run computes the region with ``0`` and ``1`` only. That is to be replaced by a region with the normal mode's nine
   values and a separate choice of two-state values; this page then changes. Treat what is said here as a list of
   differences to check, not as a promise.

.. _cycle-two-state:

What a plain ``--cycle`` run computes with
------------------------------------------

A cycle-based process computes with ``0`` and ``1`` only. Processes in the event kernel keep all nine values.

* ``'U'``, ``'X'``, ``'Z'``, ``'W'`` and ``'-'`` read as ``0``; ``'H'`` reads as ``1`` and ``'L'`` as ``0``. The end
  of the run prints how many such elements were read, counted per read: an element read as a metavalue at two wakes of
  the region counts twice, and one that comes and goes between two reads is not counted.
* The region drives only ``'0'`` and ``'1'``: a metavalue literal such as ``(others => 'X')`` assigned in region code
  gives ``'0'``. A testbench that expects an ``'X'`` or a ``'Z'`` from the design sees ``'0'``, and a released ``'H'``
  that the design copies arrives as ``'1'``. A bus model that checks read data against an expected ``'X'`` fails;
  ``to_x01`` makes ``'1'`` of both ``'1'`` and ``'H'``.
* A change between two values with the same two-state value (``'1'`` to ``'H'``, ``'U'`` to ``'X'``) is no change in
  the region: a combinational process of the region does not run for it, and makes no ``report`` or ``assert`` message
  where the normal mode makes one.
* A signal without an initial value starts at ``0``. The testbench still reads ``'U'`` at time 0, delta 0, and ``0``
  from the next delta on. Integers start at ``'left``, as in the normal mode.
* A signal the region reads that nothing drives keeps its ``'U'`` for the whole run in the normal mode, and reads as
  ``0`` in the region. A test that worked only because of that ``'U'`` (an undriven ``valid``) can now behave
  differently, or never end. The first summary line then adds ``, N inputs driven by nothing read as 0``, and
  ``--cycle-report`` names them. An unused field of a record port is counted too; it changes nothing.
* A process that compares with a metavalue literal (``= 'U'``, a ``case`` choice ``"0X"``) stays in the event kernel
  (E4101), because a two-state value never equals the literal.
* The asserts and reports of the region see these values. An assert that fires in the normal mode only because a
  value is still ``'U'`` (a register before its reset, ``unsigned(addr) < DEPTH`` on an uninitialised address) does not
  fire, and one that holds only because of a metavalue may fire. No cycle-based process gives the library warnings of
  ``numeric_std``.
* So a bug that shows as ``'X'`` or ``'U'`` in the normal mode (a register without reset, an unconnected input) cannot
  show. Run such checks in the normal mode, or with ``--cycle-exact-meta`` (:ref:`cycle-exact-meta`).

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

Keeping more of the metavalues
------------------------------

``--cycle-exact-meta`` keeps some of the normal mode's metavalues for a while, at a cost in speed. It is an interim
of cycle mode, described with its limits in :ref:`cycle-exact-meta`.

See also
--------

* :doc:`cycle-mode`: the scheduling, what joins, the report and the cache.
* :doc:`running-faster`: the speed options, with what each trades for speed.
