Errors and troubleshooting
==========================

This page says how to read a diagnostic, and what to do about the problems people run into most.
:doc:`error-codes` lists every code.

.. contents:: On this page
   :local:

Reading a diagnostic
--------------------

.. code-block:: text

    error[E1100]: `cnt` is not declared
     --> hello_error.vhd:7:12
      |
    5 |   signal count : natural;
      |          ----- similar name declared here
    6 | begin
    7 |   count <= cnt + 1;
      |            ^^^ not found in this scope
      |
      = note: LRM 12.3: without a visible declaration of this identifier, the occurrence is not legal here
      = help: a signal with a similar name exists: `count`

The first line gives the severity, a stable code and the message; then the file, line and column,
and the source with the problem marked. Notes and help lines below it, when present, say why and
what to do. A runtime error also shows the VHDL call stack.

* ``yavs --explain E1100`` prints a longer explanation of a code, and :doc:`error-codes` lists all of them.
* Diagnostics go to standard error; the reports of the simulation go to standard output.
* yavs stops at the first failing step: nothing runs after a failed ``analyze``, and an exit
  status other than 0 always comes with a diagnostic.
* ``--message-format`` and the ``-W`` options change the form and the severity: :doc:`cli-reference`.
* A misspelled name gets a suggestion: options, subcommands, option values, diagnostic codes, lint rules, libraries,
  design units, generics, enumeration literals, time units, configuration keys, and every name the debugger takes.

Exit statuses are listed in :doc:`cli-reference`. Status 3 is an internal error: please
`report it <https://github.com/ru551n/yavs/issues>`__ with the input that caused it.

Common problems
---------------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - What you see
     - What to do
   * - A test gives other results with ``--cycle`` than without
     - The list of what cycle mode changes and the checklist: :doc:`cycle-mode`; the values: :doc:`two-state`.
   * - ``yavs: command not found``
     - Put the directory of the archive on ``PATH`` (:ref:`installation`).
   * - Alpine: ``yavs: not found`` although the file exists
     - Install the ``gcompat`` package.
   * - ``error[L0001]: library `work` not found``
     - Nothing was analyzed into that ``--work`` directory yet, or the path differs from the one
       given to ``analyze``.
   * - ``L0034`` (file changed) or ``L0033`` (file moved)
     - Analyze the file again before ``run``.
   * - ``L0004``
     - The library was written by a yavs with another library format: delete the directory and
       analyze again.
   * - ``E1100``, a name is not declared
     - Check the spelling, the ``use`` clauses, and that the package was analyzed before the unit
       that uses it.
   * - ``E0011`` in a ``--std=93`` file
     - The construct needs VHDL-2008: analyze the file with ``--std=2008`` or ``2019``.
   * - Errors in verification code, such as a shared variable that is not protected
     - Try ``--relaxed`` on ``analyze`` and ``run`` (:doc:`migrating`).
   * - ``W2108``, an instance is unbound, its outputs stay ``'U'``
     - Analyze the entity of that component.
   * - The run never ends
     - A testbench with a free-running clock needs ``finish``, ``stop`` or ``--stop-time``.
       ``--hang-report`` explains a run that stops without finishing: where each process waits and
       what for, and signals that have no driver.
   * - ``W3102``
     - A process has no ``wait`` and no sensitivity list, so time cannot advance: add a ``wait``.
   * - ``E3002``, too many delta cycles
     - The error names the processes and signals that wake each other in a loop, and labels the
       declaration of each signal still changing with its value. Raise
       ``--max-deltas`` only if the design really needs more.
   * - ``E0105``
     - A time needs a unit: ``--stop-time=10us``, not ``10``.
   * - ``E0120``, tier not available
     - ``--tier`` names a compiled tier this yavs was built without (``yavs --version`` lists the
       tiers it has; ``yavs --explain E0120`` says which tier needs which build), or ``llvm`` or
       ``auto3``, which were removed: Cranelift is the compiled tier. Use a tier it has
       (``--tier=auto``, ``--tier=clift``) or ``--tier=interp``, or rebuild (:ref:`installation`).
   * - "this build of yavs needs a CPU with x86-64-v3", exit status 1
     - The x86_64 release archives without ``-compat`` in their name need AVX2, BMI1, BMI2, FMA, F16C, LZCNT
       and MOVBE; the message lists the ones this CPU lacks. Install the ``-compat`` archive of the same
       release (``yavs-<version>-x86_64-unknown-linux-gnu-compat.tar.gz`` or
       ``yavs-<version>-x86_64-pc-windows-msvc-compat.zip``), which runs on any x86-64 CPU, or build yavs from
       source (:ref:`installation`). A virtual machine may hide the features from the guest: giving it the
       host CPU model (CPU passthrough) also works.

A run that behaves differently than in another simulator may be one of the points where the
standard leaves a choice open: see :doc:`language-choices`. ``--shuffle`` runs the processes in a
random order to find testbenches that depend on it.

See also
--------

* :doc:`error-codes`: every diagnostic code.
* :doc:`faq`: questions and known limitations.
* :doc:`language-choices`: where simulators differ.
