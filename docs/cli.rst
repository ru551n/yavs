Running simulations
===================

This page shows the commands you use day to day, by task. :doc:`cli-reference` lists every command and
option with its values and defaults, and ``yavs --help`` and ``yavs COMMAND --help`` print the same.

.. contents:: On this page
   :local:

Commands
--------

.. list-table::
   :header-rows: 1
   :widths: 45 55

   * - Command
     - What it does
   * - ``yavs analyze --work=NAME:PATH FILE...``
     - Check files and store them in a library, in order; the first failing file stops.
   * - ``yavs run --work=NAME:PATH TOP``
     - Elaborate and simulate ``TOP``: an entity, ``ENTITY.ARCH`` or a configuration.
   * - ``yavs elab --work=NAME:PATH TOP``
     - Elaborate only: check that the design builds.
   * - ``yavs cov merge|report|rank ...``
     - Merge and report coverage databases (:doc:`coverage`).
   * - ``yavs lint [options] [FILE|DIR...]``
     - Report what can be proved wrong about a design before it is simulated (:doc:`lint`):
       ``--work``, ``--map``, ``--recursive``, ``--config``, ``--waivers``, ``--list-rules``,
       ``--explain RULE``, ``--json``, ``--sarif``, ``--junit``, ``--quality-report``,
       ``--sonarqube``, ``--facts``. Exit status 1 with a finding of severity error.
   * - ``yavs cache clean --work=NAME:PATH``
     - Empty the compile cache of a library (:doc:`on-disk`).
   * - ``yavs parse FILE...``
     - Only check that files parse.
   * - ``yavs --explain CODE``
     - Explain a diagnostic code (:doc:`error-codes`).
   * - ``yavs --version``
     - Print the version, whether a compiled tier is built in, and the copyright notice.

Write options as ``--name=value``. They go before or after the positional arguments.

Say which libraries to use
--------------------------

Each command names its libraries: ``--work=NAME:PATH`` for the one to analyze into or that holds the top
level, and ``--map=NAME:PATH`` for each other one. ``std`` and ``ieee`` are built in.

``--std`` on ``analyze`` is the standard of the files of that call, ``2008`` by default. A few options change how
source files are read, and go on ``analyze``, ``elab`` and ``run`` alike, because the last two read the sources
again: ``--relaxed`` (a few errors common in verification code become warnings), ``--psl`` (read PSL in
``-- psl`` comments) and ``-D NAME=VALUE`` (conditional analysis identifiers).

Control a run
-------------

* Set a generic of the top level with ``-gNAME=VALUE`` (:doc:`generics`).
* End the run at a time with ``--stop-time=10ms``. The exit status is then 0.
* Stop at a report of a lower severity with ``--exit-severity=error``. By default a ``failure`` stops the run, and
  an ``error`` report does not, but it still makes the exit status 1 at the end.
* A run that ends without ``finish`` or ``stop``: ``--hang-report`` says where each process waits and what for, labels the
  statement each stands at and lists every process that can still run.
* How many assertions failed, and where: ``--assert-summary`` prints the counts by severity and source line at
  the end, ``--message-format=json`` makes that one JSON object for CI.
* What keeps a simulation busy: ``--activity`` prints the processes that ran most often and the signals that
  changed most often, with ``--activity=FILE`` also as JSON. ``--profile`` measures time instead.
* A testbench that may depend on process order: ``--shuffle`` runs the processes in a random order and prints the
  seed, so the run can be repeated with ``--shuffle=SEED``.
* A design that may depend on the start value of a register without reset, or on what a "don't care" literal
  becomes: ``--init-unset`` and ``--init-x`` (:ref:`start-values`).

``--assert-summary`` and ``--activity`` need 0.12.0 or later.

Waves, coverage and debugging are options of ``run`` too: :doc:`waveforms`, :doc:`coverage`, :doc:`debugging`,
:doc:`interactive`. Plugins are in :doc:`vhpi`, and speed in :doc:`running-faster`.

.. _start-values:

Start values and metavalue literals
-----------------------------------

VHDL starts a ``std_ulogic`` signal or variable that has no initial value at ``'U'``. A register that has neither a
reset nor an initial value keeps the ``'U'`` (or the ``'X'`` it turns into) until something assigns it, and a test
of such a design fails in the normal mode where real hardware would start at some 0 or 1. Two options of ``run``
choose a value instead, to find out whether a design depends on one:

* ``--init-unset=VALUE`` sets the start value of every ``std_ulogic`` element (of vectors, records and arrays of
  them too) of signals, ports and process variables that have no initial value. Integers and enumerations keep their
  ``'left``, and an element whose subtype starts elsewhere (``X01`` starts at ``'X'``) keeps that value. Variables of
  subprograms, protected types and allocated objects, and shared variables, are not changed.
* ``--init-x=VALUE`` sets the value of the metavalue literals the design assigns, as a "don't care" that becomes a
  concrete value: a ``'U'``, ``'X'`` or ``'W'`` literal that is the value of a signal or variable assignment, and a
  ``'-'`` literal that is the value of such an assignment whose innermost branch is a default one (the ``others``
  choice of a ``case`` or a selected assignment, the last ``else`` of an ``if`` or a conditional assignment; a
  ``then`` branch inside an ``else`` is not). The literal may be
  qualified (``std_logic'('X')``) or the one ``others`` choice of an aggregate, nested too (``(others => 'X')``,
  ``(others => (others => '-'))``). It applies to the processes and concurrent assignments of the design below
  the top-level unit. Literals in conditions, comparisons and calls (``std_match(a, "1--0")``, ``if x = 'X'``),
  string literals, initial values, subprogram bodies and the statements of the top-level unit stay as written, so a
  ``'-'`` mask outside a default branch keeps its meaning, and so do the checks of a testbench. Verification
  components instantiated below the top-level unit are part of the design for this rule.

VALUE is ``0``, ``1``, ``random`` or ``random:SEED``. With ``random`` every element gets a bit of its own, drawn
from SEED, the object (a signal's instance path, a variable's or literal's place in the source) and the element:
the same seed gives the same bits in every tier, at every ``--threads`` and in cycle mode. Without SEED each run
draws a new one. The run prints what it chose at the start, so that a failing run can be repeated:

.. code-block:: text

    note: `--init-unset=random:418277391`: 12 signal and port elements without an initial value start at the chosen value, and so do the variables of processes; repeat the run with `--init-unset=random:418277391`

Without the options nothing changes.

In cycle mode (:doc:`cycle-mode`) both options give the cycle-based processes the values the normal mode gives:
the signals' start values come from the same elaboration, a clocked variable gets the bits of the normal mode (an
array of records may get others), and ``--init-x`` replaces the literals cycle mode would read as ``0``.
``--cycle-report`` lists the registers ``--init-unset`` changes.

Read the output
---------------

The reports of the simulation go to standard output, and diagnostics to standard error. ``--message-format=short``
gives one line per diagnostic, ``json`` one JSON object per line, for scripts and editors. :doc:`troubleshooting`
explains the form of a diagnostic.

Exit status
-----------

0 is success, 1 a failed check or runtime error, 2 a usage, library or elaboration error and 3 an internal error.
``stop(N)`` and ``finish(N)`` give N. :doc:`cli-reference` has the whole table, and :doc:`ci` what it means in a CI step.

Scripting
---------

A script or test runner calls ``yavs`` directly with an argument list, without a shell. Never ignore a non-zero
exit status.

.. code-block:: python

    import subprocess

    def yavs(*args):
        return subprocess.run(["yavs", *args], capture_output=True, text=True)

    assert yavs("analyze", "--work=lib:out/lib", "hello_tb.vhd").returncode == 0
    result = yavs("run", "--work=lib:out/lib", "-gEXPECTED=3", "hello_tb")
    print(result.returncode, result.stdout)

An argument ``@FILE`` is replaced by the arguments in FILE, one per line, taken as written (no quoting). Use it for
long file lists and long generic values.

Environment variables
---------------------

None is needed; :doc:`environment` lists all of them.

See also
--------

* :doc:`cli-reference`: every command and option.
* :doc:`project-walkthrough`: these commands in a small project.
* :doc:`generics`: generics of the top level.
* :doc:`troubleshooting`: when a command fails.
