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
       ``--sonarqube``. Exit status 1 with a finding of severity error.
   * - ``yavs cache clean --work=NAME:PATH``
     - Empty the compile cache of a library (:doc:`on-disk`).
   * - ``yavs parse FILE...``
     - Only check that files parse.
   * - ``yavs --explain CODE``
     - Explain a diagnostic code (:doc:`error-codes`).
   * - ``yavs --version``
     - Print the version and whether a compiled tier is built in.

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
* A run that ends without ``finish`` or ``stop``: ``--hang-report`` says what the waiting processes wait for.
* A testbench that may depend on process order: ``--shuffle`` runs the processes in a random order and prints the
  seed, so the run can be repeated with ``--shuffle=SEED``.

Waves, coverage and debugging are options of ``run`` too: :doc:`waveforms`, :doc:`coverage`, :doc:`debugging`,
:doc:`interactive`. Plugins are in :doc:`vhpi`, and speed in :doc:`running-faster`.

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
