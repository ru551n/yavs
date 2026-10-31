Errors and troubleshooting
==========================

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
      = help: a signal with a similar name exists: `count`

The first line gives the severity, a stable code and the message; then the file, line and column,
and the source with the problem marked. Notes and help lines below it, when present, say why and
what to do. A runtime error also shows the VHDL call stack.

* ``yavs --explain E1100`` prints a longer explanation of a code.
* Diagnostics go to standard error; the reports of the simulation go to standard output.
* yavs stops at the first failing step: nothing runs after a failed ``analyze``, and an exit
  status other than 0 always comes with a diagnostic.
* ``--message-format=short`` prints one line per diagnostic (``FILE:LINE:COLUMN: error[CODE]:
  message``), ``--message-format=json`` one JSON object per line, for editors and scripts.
* ``-Werror`` makes warnings errors; ``-Wno-CODE`` silences one warning and ``-Werror=CODE``
  makes one an error.

Exit statuses are listed in :doc:`cli`. Status 3 is an internal error: please
`report it <https://github.com/ru551n/yavs/issues>`__ with the input that caused it.

Common problems
---------------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - What you see
     - What to do
   * - ``yavs: command not found``
     - Put the directory of the archive on ``PATH`` (:doc:`install`).
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
     - Try ``--relaxed`` on ``analyze`` and ``run`` (:doc:`libraries`).
   * - ``W2108``, an instance is unbound, its outputs stay ``'U'``
     - Analyze the entity of that component (for vendor primitives, see :doc:`libraries`).
   * - The run never ends
     - A testbench with a free-running clock needs ``finish``, ``stop`` or ``--stop-time``.
       ``--hang-report`` explains a run that stops without finishing: what each waiting process
       waits for, and signals that have no driver.
   * - ``W3102``
     - A process has no ``wait`` and no sensitivity list, so time cannot advance: add a ``wait``.
   * - ``E3002``, too many delta cycles
     - The error names the processes and signals that wake each other in a loop. Raise
       ``--max-deltas`` only if the design really needs more.
   * - ``E0105``
     - A time needs a unit: ``--stop-time=10us``, not ``10``.
   * - ``E0120``, tier not available
     - ``--tier`` names a compiled tier this yavs was built without (``yavs --version`` lists the
       tiers it has; ``yavs --explain E0120`` says which tier needs which build). Use a tier it
       has or ``--tier=interp``, or rebuild (:doc:`install`).
   * - ``E0122``, ``yavs-gui`` was not found
     - Keep ``yavs-gui`` next to ``yavs`` or on ``PATH``.

A run that behaves differently than in another simulator may be one of the points where the
standard leaves a choice open: see :doc:`language-choices`. ``--shuffle`` runs the processes in a
random order to find testbenches that depend on it.
