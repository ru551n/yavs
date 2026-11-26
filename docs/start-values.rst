.. _start-values:

Start values
============

VHDL starts a ``std_ulogic`` signal or variable that has no initial value at ``'U'``. A register that has neither a
reset nor an initial value keeps the ``'U'`` (or the ``'X'`` it turns into) until something assigns it, so a
simulation shows ``'U'`` where real hardware starts at some 0 or 1. Two options of ``yavs run`` choose a value
instead, to find out whether a design depends on one. They are options of their own: they work in the normal mode,
with ``--cycle`` and with any other option.

* ``--init-unset=VALUE`` sets the start value of every ``std_ulogic`` element (of vectors, records and arrays of
  them too) of signals, ports and process variables that have no initial value. Integers and enumerations keep their
  ``'left``, and an element whose subtype starts elsewhere (``X01`` starts at ``'X'``) keeps that value. Variables of
  subprograms, protected types and allocated objects, and shared variables, are not changed.
* ``--init-x=VALUE`` sets the value of the metavalue literals the design assigns, as a "don't care" that becomes a
  concrete value: a ``'U'``, ``'X'`` or ``'W'`` literal that is the value of a signal or variable assignment, and a
  ``'-'`` literal that is the value of such an assignment whose innermost branch is a default one (the ``others``
  choice of a ``case`` or a selected assignment, the last ``else`` of an ``if`` or a conditional assignment; a
  ``then`` branch inside an ``else`` is not). The literal may be qualified (``std_logic'('X')``) or the one
  ``others`` choice of an aggregate, nested too (``(others => 'X')``, ``(others => (others => '-'))``). It applies
  to the processes and concurrent assignments of the design below the top-level unit. Literals in conditions,
  comparisons and calls (``std_match(a, "1--0")``, ``if x = 'X'``), string literals, initial values, subprogram
  bodies and the statements of the top-level unit stay as written, so a ``'-'`` mask outside a default branch keeps
  its meaning, and so do the checks of a testbench. Verification components instantiated below the top-level unit
  are part of the design for this rule. A ``'-'`` in a default branch is replaced like the others: what it assigns
  is no longer a "don't care" but the chosen value.

VALUE is ``0``, ``1``, ``random`` or ``random:SEED``. Without the options nothing changes.

Verify the reset
----------------

A design that resets every register it needs gives the same results whatever its registers start at. Run the suite
three times, with ``--init-unset=0``, ``--init-unset=1`` and ``--init-unset=random``: a test that passes in one and
fails in another depends on a register that starts without a reset. Add ``--init-x`` with the same value to also
check that no "don't care" the design assigns reaches a result.

With VUnit and vunit-yavs, the sim options ``yavs.init_unset`` and ``yavs.init_x`` (:doc:`vunit-yavs`) set the
options, and three configurations of each testbench run the three variants side by side:

.. code-block:: python

    for value in ("0", "1", "random"):
        for tb in lib.get_test_benches():
            tb.add_config(
                name=f"init_{value}",
                sim_options={"yavs.init_unset": value, "yavs.init_x": value},
            )

Each test then runs once per configuration. A wrong value of either option fails when it is set and names the
closest valid one.

Repeat a run
------------

With ``random`` every element gets a bit of its own, drawn from SEED and from where the element is: a signal's
instance path and element; a process variable's name and place in the source together with the instance path of its
process, so two instances of one entity get bits of their own; a literal's place in the source, the same in every
instance. A place in the source is the file's name without its directory, the line and the column, so the bits do
not change with where the sources are checked out, the system's path separators or line ends. Two files of the same
name in different directories give literals at the same line and column the same bits. Nothing else enters:
not the order of elaboration or of the processes, the threads or the cache.

So one seed gives the same values in every run of the same design: in every tier, at every ``--threads``, with
``--shuffle``, in the normal mode and in cycle mode (where both modes give the literal a value, see below), from a
cold or a warm cache, on every system. Without SEED each run draws a new one from the time. The run prints what it
chose at the start:

.. code-block:: text

    note: `--init-unset=random:513994796`: 5181 signal and port elements without an initial value start at the chosen value, and so do the variables of processes; repeat the run with `--init-unset=random:513994796`

To repeat a failing run, pass the printed value: ``--init-unset=random:513994796`` (in VUnit, the test's output
holds the note; set ``yavs.init_unset`` to ``"random:513994796"``).

``--init-x`` prints a note of its own. When both options are ``random`` without a seed, each draws a seed of its own,
and the two are different. To repeat the run you need **both** printed values; with only one of them, the other option
draws new bits:

.. code-block:: text

    note: `--init-unset=random:667830990`: 21 signal and port elements without an initial value start at the chosen value, and so do the variables of processes; repeat the run with `--init-unset=random:667830990`
    note: `--init-x=random:407466518`: metavalue literals the design assigns take the chosen value; repeat the run with `--init-x=random:407466518`

.. code-block:: bash

    yavs run --init-unset=random:667830990 --init-x=random:407466518 --work=work:build/work tb

In VUnit, set both sim options, for the test or in a configuration of its own:

.. code-block:: python

    tb.add_config(
        name="repeat",
        sim_options={"yavs.init_unset": "random:667830990", "yavs.init_x": "random:407466518"},
    )

The seed is read when the run starts, never compiled: runs that differ only in the seed share the compiled code and
the compilation cache, in the normal mode and in cycle mode. A second seed starts at once, and the VUnit
configurations above that differ only in the seed share one build. Only the choice of ``0``, ``1`` or ``random``
is compiled in.

Find the registers without a reset
----------------------------------

``yavs run --cycle --cycle-report`` lists the registers ``--init-unset`` changes: the register signals and clocked
variables without an initial value that a cycle-based clocked process writes without an asynchronous reset branch.
A synchronous reset is not looked for, so a register with one is listed too, and processes that stay in the event
kernel are not listed. The list shows the first 100 names. The normal mode prints only how many signal elements
had no initial value (the note above).

In cycle mode
-------------

In cycle mode (:doc:`cycle-mode`) ``--init-unset`` gives the cycle-based processes the values the normal mode
gives: the signals' start values come from the same elaboration, and a clocked variable gets the bits of the normal
mode (an array of records may get others).

``--init-x`` gives the literals above the normal mode's value too. With ``--two-state`` it goes further, in cycle
mode and in the normal mode alike: its scope (the design under test by default) computes with ``0`` and ``1`` only, so every
metavalue literal it assigns there, also one in a string or a ``case`` alternative, takes the value of ``--init-x``,
``0`` without it. A metavalue literal it compares with keeps its meaning: a ``0`` or ``1`` never equals ``'U'``,
``'X'``, ``'Z'`` or ``'W'``, so ``x = 'X'`` is false, ``assert clk /= 'U'`` holds, and a ``case`` choice ``"0X"`` is
never taken; a ``'-'`` in ``std_match``, ``?=`` or a ``case?`` choice still matches anything. The processes outside the
scope keep their literals' meaning, as in the normal mode. ``--cycle-report`` counts the literals per
process.
