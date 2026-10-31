vunit-yavs options
==================

The ``vunit-yavs`` package (``pip install vunit-yavs``) adds yavs to VUnit as a simulator; :doc:`vunit` is the guide.
This page lists its options. The options of the ``yavs``
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
     - ``yavs run --threads=N``: the most threads one simulation uses (with the token server, ``-p N`` but at most 4 when not set). An integer of at least 1.
       It overrides ``--yavs-max-threads`` for a test. Not passed under ``--yavs-debug`` or to
       ``elab``. A ``--threads`` in ``yavs.sim_flags`` comes after it and wins.
   * - ``yavs.cov_kind``
     - ``yavs run --cov-kind=LIST``: the kinds of coverage to count (``statement``, ``branch``,
       ``toggle``, ``condition``, ``expression``). Used with VUnit's own ``enable_coverage``
       sim option, which makes every run write a coverage database (:ref:`vunit-coverage`).
   * - ``yavs.init_files.after_load``
     - Files of console commands sourced after each ``vunit_load`` (:doc:`interactive`).
   * - ``yavs.init_files.before_run``
     - Files of console commands sourced before each ``vunit_run``.
   * - ``yavs.init_file.gui``
     - A file of console commands sourced by ``vunit_user_init`` and at the start of
       ``--yavs-interactive``.

Command-line flags
------------------

Flags of the run script (``python run.py ...``), next to VUnit's own:

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Flag
     - What it does
   * - ``--yavs-debug[=PORT]``
     - Runs the one selected test under ``yavs run --dap=PORT`` and waits for a debugger on
       ``127.0.0.1:PORT`` (default ``4711``, ``0`` picks a free port). Write the port as
       ``--yavs-debug=PORT``: a test name after a bare ``--yavs-debug`` is read as the port.
       Not with ``--elaborate``, ``-p`` above 1 or more than one simulation.
   * - ``--yavs-interactive``
     - Runs the one selected test at the yavs console (``yavs run --interactive``, :doc:`interactive`),
       with the ``vunit_*`` commands. Not with ``--yavs-debug``, ``--elaborate``, ``-p`` above 1
       or more than one simulation.
   * - ``--yavs-surfer[=HOST:PORT]``
     - Shows the waves of the one selected test in Surfer while it runs (``yavs run --surfer``,
       :doc:`waveforms`). Not with ``-p`` above 1 or more than one simulation.
   * - ``--yavs-max-threads N``
     - ``yavs.max_threads`` for every test that does not set it.
   * - ``--yavs-lint``
     - After compiling, runs ``yavs lint`` (:doc:`lint`) on every library of the project that is not
       external; a finding of severity error fails the compile step. VUnit's own libraries are among
       them: to leave them out, put ``exclude: ['**/vunit/vhdl/**']`` in ``yavs-lint.yaml`` in the
       directory you run in or an ancestor.
   * - ``--yavs-cov``
     - Collects coverage in every simulation, as ``enable_coverage`` does for the tests it is set
       for. Not with ``--yavs-debug``.
   * - ``--yavs-cov-html DIR``
     - ``results.merge_coverage()`` also writes the HTML coverage report into DIR.
   * - ``--yavs-cov-fail-under PCT``
     - ``results.merge_coverage()`` prints the coverage report, and the run exits with status 1
       (after VUnit has written its reports) when the total coverage is below PCT percent.
   * - ``--yavs-jobserver``
     - With ``-p N`` above 1: lends the CPU slots that no test uses to the running simulations
       through a token server (:doc:`threads`).

Without ``yavs.max_threads`` or ``--yavs-max-threads`` no thread count is passed and yavs runs
with its default, unless the token server runs.

Compiling
---------

The package analyzes consecutive files of one library, standard and flags in one ``yavs analyze`` call. A VUnit
release with ``--batch-compile`` and ``--parallel-compile`` does the compiling itself when you give those flags:
yavs accepts compiling into one library from several processes and several files in a call, and the file
names go on the command line.

.. _vunit-coverage:

Coverage
--------

.. code-block:: python

   vu.set_sim_option("enable_coverage", True)      # or run with --yavs-cov
   vu.set_sim_option("yavs.cov_kind", ["statement", "branch"])
   vu.main(post_run=lambda results: results.merge_coverage("coverage"))

Each simulation writes a database named after its test. ``merge_coverage("coverage")`` merges
them into ``coverage.ycov``. With ``--yavs-cov-html``, ``--yavs-cov-fail-under`` or arguments for
``yavs cov report`` (``merge_coverage("coverage", args=["--cobertura", "c.xml"])``) it then
reports on the merged database. The reports, exclusions and numbers are in :doc:`coverage`;
a CI gate is ``python run.py --yavs-cov --yavs-cov-fail-under 85``.

Run scripts you do not write
----------------------------

A third-party ``run.py`` knows nothing about yavs. Run it through a small wrapper that adds the package
(``vu.add_package("vunit-yavs", allow_setup=True)``) after the script creates its ``VUnit`` object, and
note:

* The simulator registry is global to the Python process: a script that creates two ``VUnit`` objects
  can add the package only once (the second ``add_package`` fails with "already registered"), so the
  wrapper catches that error for the second object.
* A script with a command line of its own has no ``--yavs-max-threads``: set the environment variable
  ``VUNIT_YAVS_MAX_THREADS`` instead.
* ``yavs.max_threads`` is one integer, not a list: ``set_sim_option("yavs.max_threads", 4)``. VUnit's
  ``overwrite=False`` appends to a list option and fails for an integer.
* Scripts that turn on relaxed mode or VHDL-2019 only for the simulators they know need it told for
  yavs: ``set_compile_option("yavs.global_flags", ["--relaxed"])`` and the same as a sim option, and
  ``VUNIT_VHDL_STANDARD=2019`` in the environment.

Environment variables
---------------------

``VUNIT_SIMULATOR=yavs`` selects the simulator. ``VUNIT_YAVS_MAX_THREADS=N`` is ``--yavs-max-threads`` for a run script that
parses its own command line. ``VUNIT_YAVS_PATH`` names the directory of the
``yavs`` executable (otherwise it is found on ``PATH``). :doc:`environment` lists all of them.

See also
--------

* :doc:`vunit`: the guide, from installing to running.
* :doc:`threads`: threads and the token server.
* :doc:`cli-reference`: the options of the ``yavs`` executable.
