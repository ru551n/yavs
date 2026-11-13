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
     - ``yavs elab`` and ``yavs run`` (sim option). A native library goes here with its argument layout, for example
       ``--load=./libhelpers.so:args=fat-pointer`` (:doc:`vhpi`); a package uses
       ``native_library_hooks(library, args="fat-pointer")`` (:doc:`vunit`).
   * - ``yavs.sim_flags``
     - ``yavs run`` (sim option).
   * - ``yavs.max_threads``
     - ``yavs run --threads=N``: the most threads one simulation uses, an integer of at least 1, or ``"auto"`` for
       ``--threads=auto`` (more threads only while they make the run faster; the default with the token server).
       It overrides ``--yavs-max-threads`` for a test. Not passed under ``--yavs-debug`` or to
       ``elab``. A ``--threads`` in ``yavs.sim_flags`` comes after it and wins.
   * - ``yavs.tier_compile``
     - ``yavs run --tier-compile=MODE``: ``"wait"`` (yavs's default: a simulation waits while a function compiles; a tier-up
       thread still prepares functions alongside it, so a run can use somewhat more cores than its threads for short
       periods) or ``"background"`` (functions compile on a thread of their
       own at the lowest system priority, alongside the run: a test that compiles a lot may end sooner when there are
       idle cores) (:doc:`running-faster`). Not passed to ``elab``. A ``--tier-compile`` in ``yavs.sim_flags``
       wins.
   * - ``yavs.cov_embed_sources``
     - ``yavs run --cov-embed-sources``: store the source text in the database, so the HTML report needs no
       checkout. The source text then travels with the database (:doc:`coverage`).
   * - ``yavs.cov_kind``
     - ``yavs run --cov-kind=LIST``: the kinds of coverage to count (``statement``, ``branch``,
       ``toggle``, ``condition``, ``assert``, ``expression``, ``combination``, ``fsm``). Used with VUnit's own ``enable_coverage``
       sim option, which makes every run write a coverage database (:ref:`vunit-coverage`). No kind is on by default:
       ``enable_coverage`` without this option, ``--yavs-cov-kind`` or ``--yavs-cov=LETTERS`` fails the simulation with a message that names them.
   * - ``yavs.init_files.after_load``
     - Files of console commands sourced after each ``vunit_load`` (:doc:`interactive`).
   * - ``yavs.init_files.before_run``
     - Files of console commands sourced before each ``vunit_run``.
   * - ``yavs.init_file.gui``
     - A file of console commands sourced by ``vunit_user_init`` and at the start of
       ``--yavs-interactive``.

Command-line flags
------------------

Flags of the run script (``python run.py ...``), next to VUnit's own. A combination the table rules
out stops the run script with one ``error:`` line that says why and what to drop, and status 1:

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Flag
     - What it does
   * - ``--yavs-debug[=PORT]``
     - Runs the one selected test under ``yavs run --dap=PORT`` and waits for a debugger on
       ``127.0.0.1:PORT`` (default ``4711``, ``0`` picks a free port). Write the port as
       ``--yavs-debug=PORT``: a test name after a bare ``--yavs-debug`` is read as the port.
       Not with ``--yavs-cov``, ``--elaborate``, ``-p`` above 1 or more than one simulation.
   * - ``--yavs-interactive``
     - Runs the one selected test at the yavs console (``yavs run --interactive``, :doc:`interactive`),
       with the ``vunit_*`` commands. Not with ``--yavs-debug``, ``--yavs-cov``, ``--elaborate``,
       ``-p`` above 1 or more than one simulation.
   * - ``--yavs-surfer[=HOST:PORT]``
     - Shows the waves of the one selected test in Surfer while it runs (``yavs run --surfer``,
       :doc:`waveforms`). Combines with ``--yavs-debug`` and ``--yavs-interactive``, where the cursor follows the stops.
       Not with ``--elaborate``, ``-p`` above 1 or more than one simulation.
   * - ``--yavs-max-threads N``
     - ``yavs.max_threads`` for every test that does not set it.
   * - ``--yavs-lint``
     - After compiling, runs ``yavs lint`` (:doc:`lint`) on every library of the project that is not
       external; a finding of severity error fails the compile step. VUnit's own libraries are among
       them: to leave them out, put ``exclude: ['**/vunit/vhdl/**']`` in ``yavs-lint.yaml`` in the
       directory you run in or an ancestor. ``file_list`` in that file is not read: the project says
       which files a library has. The configuration is read once before any library is checked: an
       unknown key, rule or rule group in it or in the waiver file is error E0134 and fails the compile step.
   * - ``--yavs-cov=LETTERS``
     - Collects coverage in every simulation, as ``enable_coverage`` does for the tests it is set
       for, of the kinds the letters name (``--yavs-cov=sbt``): ``s`` statement, ``b`` branch, ``c`` condition,
       ``e`` expression, ``t`` toggle, ``f`` fsm, ``a`` assert, ``m`` combination. No kind is on by default.
       Write the letters with ``=``. Not with ``--yavs-debug`` or ``--yavs-interactive``.
   * - ``--yavs-cov-directives MODE``
     - ``apply`` or ``ignore``: what ``yavs analyze``, ``elab`` and ``run`` do with the
       ``-- yavs coverage exclude`` comments in the sources (``--coverage-directives``, :doc:`coverage`).
       Without it they are checked when compiling, as yavs does by default.
   * - ``--yavs-cov-kind KIND,...``
     - The kinds of coverage to count, as ``yavs run --cov-kind`` (``fsm``, ``assert``, ``condition`` and the others), for tests without a ``yavs.cov_kind`` sim option. The long form of ``--yavs-cov=LETTERS``, not given with it; it also turns coverage on.
   * - ``--yavs-cov-html DIR``
     - ``results.merge_coverage()`` also writes the HTML coverage report into DIR.
   * - ``--yavs-cov-fail-under PCT``
     - ``results.merge_coverage()`` prints the coverage report, and the run exits with status 1
       (after VUnit has written its reports) when the total coverage is below PCT percent. Also
       per kind: ``statement:90,branch:80``.
   * - ``--yavs-cov-cobertura FILE``, ``--yavs-cov-json FILE``, ``--yavs-cov-lcov FILE``, ``--yavs-cov-sonarqube FILE``
     - ``results.merge_coverage()`` also writes that export (:doc:`coverage`). In a script:
       ``merge_coverage("coverage", args=["--lcov", "c.info"])`` and so on.
   * - ``--yavs-cov-embed-sources``
     - Passes ``--cov-embed-sources`` to every simulation with coverage, as ``yavs.cov_embed_sources`` does for the
       tests it is set for.
   * - ``--yavs-cov-below PCT``
     - The report of ``results.merge_coverage()`` limits its table of instances to those below PCT
       percent, worst first (``--below``, :doc:`coverage`).
   * - ``--yavs-cov-detailed``
     - That report also lists what never ran, under each instance (``--detailed``, :doc:`coverage`).
   * - ``--yavs-cov-exclude FILE``
     - ``results.merge_coverage()`` reports without the items this exclusion file lists
       (``yavs cov report --exclude``). Repeatable: the rules of all files add up.
   * - ``--yavs-cov-config FILE``
     - ``results.merge_coverage()`` reads exclusion files, root and kinds from this YAML file
       (``yavs cov report --config``). No file is read without it.
   * - ``--yavs-jobserver``
     - With ``-p N`` above 1 (``-p 0``, one job per CPU, counts as the CPU count): lends the CPU slots that no test uses to the running simulations
       through a token server (:doc:`threads`).

Without ``yavs.max_threads`` or ``--yavs-max-threads`` no thread count is passed and yavs runs
with its default, one thread, unless the token server runs: then every simulation gets ``--threads=auto``.

Compiling
---------

The package analyzes consecutive files of one library, standard and flags in one ``yavs analyze`` call. A VUnit
release with ``--batch-compile`` and ``--parallel-compile`` (5.0.0.dev16 does not have them) does the compiling itself
when you give those flags:
yavs accepts compiling into one library from several processes and several files in a call, and the file
names go on the command line.

.. _vunit-coverage:

Coverage
--------

.. code-block:: python

   vu.set_sim_option("enable_coverage", True)      # or run with --yavs-cov=sb
   vu.set_sim_option("yavs.cov_kind", ["statement", "branch"])     # no kind is on by default
   vu.main(post_run=lambda results: results.merge_coverage("coverage"))

Each simulation writes a database named after its test. ``merge_coverage("coverage")`` merges
them into ``coverage.ycov``. With ``--yavs-cov-html``, ``--yavs-cov-fail-under`` or arguments for
``yavs cov report`` (``merge_coverage("coverage", args=["--cobertura", "c.xml"])``) it then
reports on the merged database. The reports, exclusions and numbers are in :doc:`coverage`;
a CI gate is ``python run.py --yavs-cov=sbt --yavs-cov-fail-under 85``. In a script, what a report does goes through ``args``: ``merge_coverage("coverage", args=["--below=100", "--detailed"])`` is the
"what is left to do" view.

A run script passes exclusions in ``args``, and a person adds more on the command line:

.. code-block:: python

   vu.main(post_run=lambda results: results.merge_coverage(
       "coverage", args=["--exclude", "waived.yaml", "--exclude", "generated.txt", "--config", "coverage.yaml"]))

``--yavs-cov-exclude`` and ``--yavs-cov-config`` go after ``args``. A bad file fails the report with its own error, and
the run exits with status 1 after VUnit has written its reports.

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
