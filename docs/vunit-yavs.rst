vunit-yavs options
==================

The ``vunit-yavs`` package (``pip install vunit-yavs``) adds yavs to VUnit as a simulator; see
:doc:`libraries` for the setup. This page lists its options. The options of the ``yavs``
executable itself are in :doc:`cli-reference`. The names below are checked against the package
by its tests.

Sim and compile options
-----------------------

Set them with ``vu.set_sim_option("yavs.sim_flags", [...])`` or ``set_compile_option``, on the
project, a library, a testbench or a test. Flags are lists of strings, one argument each, written
as ``--name=value``.

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Option
     - Passed to
   * - ``yavs.global_flags``
     - ``yavs analyze``, ``yavs elab`` and ``yavs run``. A compile option for ``analyze`` and a
       sim option for ``elab`` and ``run``.
   * - ``yavs.a_flags``
     - ``yavs analyze`` (compile option).
   * - ``yavs.elab_flags``
     - ``yavs elab`` and ``yavs run`` (sim option).
   * - ``yavs.sim_flags``
     - ``yavs run`` (sim option).
   * - ``yavs.max_threads``
     - ``yavs run --threads=N``: the most threads one simulation uses. An integer of at least 1.
       It overrides ``--yavs-max-threads`` for a test. Not passed under ``--yavs-debug`` or to
       ``elab``. A ``--threads`` in ``yavs.sim_flags`` comes after it and wins.

Command-line flags
------------------

Flags of the run script (``python run.py ...``), next to VUnit's own:

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Flag
     - What it does
   * - ``-g``, ``--gui``
     - Opens the run in the graphical debugger (``yavs run --gui``).
   * - ``--yavs-debug[=PORT]``
     - Runs the one selected test under ``yavs run --dap=PORT`` and waits for a debugger on
       ``127.0.0.1:PORT`` (default ``4711``, ``0`` picks a free port). Write the port as
       ``--yavs-debug=PORT``: a test name after a bare ``--yavs-debug`` is read as the port.
       Not with ``--gui``, ``--elaborate``, ``-p`` above 1 or more than one simulation.
   * - ``--yavs-max-threads N``
     - ``yavs.max_threads`` for every test that does not set it.
   * - ``--yavs-jobserver``
     - With ``-p N`` above 1: lends the CPU slots that no test uses to the running simulations
       through a token server (:ref:`vunit-token-server`).

Without ``yavs.max_threads`` or ``--yavs-max-threads`` no thread count is passed and yavs runs
with its default, unless the token server runs.

Environment variables
---------------------

``VUNIT_SIMULATOR=yavs`` selects the simulator. ``VUNIT_YAVS_PATH`` names the directory of the
``yavs`` executable (otherwise it is found on ``PATH``).
