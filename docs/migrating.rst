Moving from another simulator
=============================

This page is for you if you know another VHDL simulator and want to know what is different in yavs:
the concepts first, then the same tasks in a table.

.. contents:: On this page
   :local:

The same tasks in yavs
----------------------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Task
     - In yavs
   * - Create a library
     - Nothing to do: ``analyze`` creates the directory of ``--work=NAME:PATH``.
   * - Map a logical library to a directory
     - ``--work=NAME:PATH`` and ``--map=NAME:PATH`` on each command. There is no setup file.
   * - Compile a file
     - ``yavs analyze --work=NAME:PATH FILE...``
   * - Choose the language standard
     - ``--std=93``, ``--std=2008`` (the default) or ``--std=2019`` on ``analyze``.
   * - Elaborate without simulating
     - ``yavs elab --work=NAME:PATH TOP``
   * - Simulate
     - ``yavs run --work=NAME:PATH TOP``. It elaborates too: there is no separate step to run first.
   * - Pick an architecture or a configuration
     - ``ENTITY.ARCH`` or the name of a configuration, in place of ``TOP``.
   * - Set a generic of the top level
     - ``-gNAME=VALUE`` (:doc:`generics`).
   * - Stop at a time
     - ``--stop-time=10us``
   * - Stop on an error report
     - ``--exit-severity=error``
   * - Dump waveforms
     - ``--wave=FILE.fst``. The format is FST (:doc:`waveforms`).
   * - Run a command script
     - ``--do "COMMANDS"`` or ``--do "source FILE"`` (:doc:`interactive`, a release newer than 0.11.0).
   * - Open a debugger
     - VS Code or another Debug Adapter Protocol client with ``--dap=PORT`` (:doc:`debugging`), or a
       prompt with ``--interactive`` (a release newer than 0.11.0).
   * - Collect coverage
     - ``--cov=FILE`` on ``run``, then ``yavs cov report`` (:doc:`coverage`).
   * - Silence a warning
     - ``-Wno-CODE``. ``-Werror`` makes warnings errors.
   * - Find out what a message means
     - ``yavs --explain CODE`` (:doc:`error-codes`).
   * - Compile the vendor libraries
     - Plain ``analyze``, there is no special command (:doc:`vendor-libraries`).
   * - Run a regression
     - VUnit with the ``vunit-yavs`` package (:doc:`vunit`), or call ``yavs`` from a script (:doc:`cli`).

Work library and mapping
------------------------

A library is a directory, and each command says where it is. There is no project file, no ``.ini``
file and no setup command, so a script holds everything it needs:

.. code-block:: bash

    yavs analyze --work=mylib:build/mylib pkg.vhd dut.vhd
    yavs analyze --work=work:build/work --map=mylib:build/mylib tb.vhd
    yavs run --work=work:build/work --map=mylib:build/mylib tb

``std`` and ``ieee`` are built in. They cannot be mapped to a library of your own. Vendor and
verification libraries are analyzed from their sources; there are no precompiled ones.

Analyze, then run
-----------------

A library does not hold compiled code. It records which files it holds, with a hash of each. So:

* ``run`` needs the source files to be where ``analyze`` read them, unchanged. A changed file is
  error ``L0034``, a missing one ``L0033``.
* ``run`` gets the options that change how sources are read, as ``analyze`` does: ``--relaxed``,
  ``--psl`` and ``-D NAME=VALUE``. It does not get ``--std``, which is recorded per file.
* yavs keeps what it can reuse in a cache beside the library (:doc:`on-disk`). You do not manage it.

Top level and generics
----------------------

The top level is a positional argument: ``yavs run --work=work:build/work tb``. Only generics of the
top level can be set from the command line, by their simple name; a dotted path is not looked up.
Other simulators take values in several forms, so read :doc:`generics` for the forms yavs takes. A name
the top level does not have is warning ``W2105``, and the run goes on.

Time
----

* The time resolution is 1 fs unless ``--resolution`` says otherwise, whatever the source uses.
  Reports show the time in femtoseconds, also with a coarser ``--resolution``: ``32000000fs+0`` is
  32 ns in delta cycle 0.
* With ``--resolution=ns`` a time literal in a smaller unit, such as ``1 ps``, is an error before time
  0, also in code that never runs. Other simulators may round it instead.
* ``--stop-time`` needs a unit: ``10ns`` or ``"10 ns"``. Without one it is error ``E0105``.
* A run without ``--stop-time`` goes on until no event is left or the design calls ``finish`` or
  ``stop``. A free-running clock therefore never ends: ``yavs`` has no default run length.

Where yavs is stricter
----------------------

Where the standard is clear yavs follows it and stops early, so some code that another simulator lets
through is an error here. The common cases:

* A shared variable of a type that is not protected, a pure function that calls an impure one, and an
  ``others`` aggregate assigned to an object with an unconstrained subtype. See below for
  ``--relaxed``.
* ``--std=93`` rejects what VHDL-2008 added (``E0011``) and what it relaxed (``E1007``).
* Division by zero in a constant range bound, and a type, subprogram or package in a parameter list.
* No error is downgraded on its own, and a failed step never lets the next one run.

Where yavs is more permissive
-----------------------------

Where the standard is open, yavs takes the reading that lets the most designs run:

* Files of different standards can share one design (:doc:`mixing-standards`).
* Package instances in any declarative part, including processes and subprograms.
* ``std_logic_arith`` and the other Synopsys packages are in ``ieee``, with no option.
* A static index out of range, or a constant overflow, is a warning when the code may be dead. The
  check still runs if the code runs.

Code that passes in yavs for these reasons can still fail elsewhere. :doc:`language-choices` lists the
differences you are most likely to see and what to do about each. The full catalogue, with a test case
for each difference and what each simulator does with it, is
`simulator-discrepancies.md <https://github.com/ru551n/yavs/blob/main/docs/design/simulator-discrepancies.md>`__.

Relaxed mode
------------

``--relaxed`` turns a few errors into warnings: a shared variable of a type that is not protected
(``W1904``), a pure function that calls an impure one (``W1830``), an ``others`` aggregate assigned to
an object with an unconstrained subtype (``W1325``) and a signal parameter's actual that is not a
static name (``W1361``). Pass it to ``analyze`` and also to ``run`` and ``elab``, which read the files
again.

Process order
-------------

Processes that wake in the same cycle run in one fixed order, the same at any ``--threads``. A
testbench that relies on the order of another simulator can give another result. ``--shuffle`` runs
them in a random order, to find such a dependency; pass values through signals to remove it.

Exit status
-----------

A script can rely on the exit status. 0 means success, 1 a failed assertion or a runtime error, 2 a
usage, library or elaboration error, and 3 an internal error. ``stop(N)`` and ``finish(N)`` give N. An
``error`` report does not stop the run, but the exit status is 1 at the end. A ``failure`` report stops
it. :doc:`cli-reference` has the full table.

See also
--------

* :doc:`getting-started`: the first simulation.
* :doc:`project-walkthrough`: a project with several libraries.
* :doc:`language-choices`: where simulators differ, and what to do.
* :doc:`faq`: what yavs does not do.
