.. This page is generated from the command-line definition (crates/yavs/src/reference.rs). Do
.. not edit it. After a change to an option, default, value or help text regenerate it with:
..
..     YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference

Command-line reference
======================

Every command and option of ``yavs``, as ``yavs --help`` and ``yavs COMMAND --help`` print them.
:doc:`cli` is the task-oriented guide; the options of ``vunit-yavs`` are in :doc:`vunit-yavs`.

Write an option with a value as ``--name=value``. Options go before or after the positional
arguments. An argument ``@FILE`` is replaced by the arguments in FILE, one per line, taken as
written, so that long file lists and generic values need no quoting.

Commands
--------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Command
     - What it does
   * - ``yavs parse [OPTIONS] FILES...``
     - Check that VHDL files parse
   * - ``yavs analyze [OPTIONS] --work=NAME:PATH FILE...``
     - Analyze one VHDL file into a library
   * - ``yavs elab [OPTIONS] --work=NAME:PATH TOP``
     - Elaborate a design (no simulation)
   * - ``yavs run [OPTIONS] --work=NAME:PATH TOP [+NAME[=VALUE]...]``
     - Elaborate and simulate a design
   * - ``yavs cov merge --output=FILE DB...``
     - Add up the counts of several coverage databases into one
   * - ``yavs cov report [OPTIONS] DB...``
     - Print a text summary of coverage databases (merged), and write other reports
   * - ``yavs cov rank [OPTIONS] DB...``
     - Rank the tests of merged databases: a minimal set reaching the same coverage, and what each test alone covers
   * - ``yavs lint [OPTIONS] [FILE|DIR...]``
     - Report what can be proved wrong about a design before it is simulated
   * - ``yavs cache clean [OPTIONS] --work=NAME:PATH``
     - Remove the cache entries of libraries
   * - ``yavs --explain CODE``
     - Print the explanation of a diagnostic code.
   * - ``yavs --version``
     - Print the version and whether a compiled tier is built in.

Options of every command
------------------------

.. program:: yavs

.. option:: --color=auto|always|never

   When to colour diagnostics: ``auto`` colours on a terminal unless ``NO_COLOR`` is set.

   Default: ``auto``.

.. option:: --message-format=human|short|json

   How to print diagnostics on standard error: ``human`` (source snippets), ``short`` (one line each, ``FILE:LINE:COL: severity[CODE]: message``) or ``json`` (one JSON object per diagnostic).

   Default: ``human``.

.. option:: --encoding=auto|latin1|utf8

   How to read source files: UTF-8 when valid and ISO-8859-1 otherwise (auto), always ISO-8859-1 bytes (latin1), or UTF-8 only (utf8).

   Default: ``auto``.

.. option:: --explain=CODE

   Print the long explanation of a diagnostic code, such as ``E0002``, and exit.

.. option:: -h, --help

   Print the help of the command and exit.

.. option:: -V, --version

   Print the version and exit.

``--color``, ``--message-format`` and ``--encoding`` are accepted before and after the subcommand; ``--explain`` stands alone.

``yavs parse``
--------------

.. program:: yavs parse

Check that VHDL files parse.

.. code-block:: text

   yavs parse [OPTIONS] FILES...

Arguments:

* ``FILES`` (one or more): The files to check.

.. option:: --std=93|2008|2019

   Language standard.

   Default: ``2008``.

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``.

   May be repeated.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas.

``yavs analyze``
----------------

.. program:: yavs analyze

Analyze one VHDL file into a library.

.. code-block:: text

   yavs analyze [OPTIONS] --work=NAME:PATH FILE...

Arguments:

* ``FILE`` (one or more): The files, analyzed in this order; the first failing file stops the batch.

.. option:: --std=93|2008|2019

   Language standard of this file.

   Default: ``2008``.

.. option:: --work=NAME:PATH

   The library analyzed into, ``NAME:PATH``.

   Required.

.. option:: --map=NAME:PATH

   Another library, ``NAME:PATH`` (repeatable).

   May be repeated.

.. option:: --relaxed

   Turn some errors into warnings: a shared variable of a non-protected type, a pure function that calls an impure one, an ``others`` aggregate for an unconstrained target, a non-static actual of a signal parameter.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas (native PSL statements are always read).

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``.

   May be repeated.

.. option:: -W FLAG

   Warning control: ``-Werror`` (every warning is an error), ``-Werror=CODE``, ``-Wno-CODE`` (do not print it), ``-WCODE`` (stays a warning under ``-Werror``).

   May be repeated.

.. option:: --bootstrap

   Allow ``std`` and ``ieee`` as ``--work`` or ``--map``: for building the standard libraries from their sources.

.. option:: --no-cache

   Neither read nor write the analysed-unit cache of the libraries.

.. option:: --std-cache=DIR|off

   Directory of the std/ieee units shared by all libraries, or ``off`` (default: ``$YAVS_STD_CACHE``, else ``yavs/std`` in the user's cache directory).

``yavs elab``
-------------

.. program:: yavs elab

Elaborate a design (no simulation).

.. code-block:: text

   yavs elab [OPTIONS] --work=NAME:PATH TOP

Arguments:

* ``TOP``: ``ENTITY``, ``ENTITY.ARCH`` or ``CONFIGURATION`` in the work library.

.. option:: --std=93|2008|2019

   Design revision of std/ieee; default: the newest standard in the design's closure.

.. option:: --work=NAME:PATH

   The library holding the top level, ``NAME:PATH``.

   Required.

.. option:: --map=NAME:PATH

   Another library, ``NAME:PATH`` (repeatable).

   May be repeated.

.. option:: --relaxed

   Turn some errors into warnings: a shared variable of a non-protected type, a pure function that calls an impure one, an ``others`` aggregate for an unconstrained target, a non-static actual of a signal parameter.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas (native PSL statements are always read).

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``: the same definitions as when the files were analyzed, because the sources are read again.

   May be repeated.

.. option:: -W FLAG

   Warning control: ``-Werror`` (every warning is an error), ``-Werror=CODE``, ``-Wno-CODE`` (do not print it), ``-WCODE`` (stays a warning under ``-Werror``).

   May be repeated.

.. option:: --no-cache

   Neither read nor write the compilation cache of the work library.

.. option:: --std-cache=DIR|off

   Directory of the std/ieee units shared by all libraries, or ``off`` (default: ``$YAVS_STD_CACHE``, else ``yavs/std`` in the user's cache directory).

.. option:: --no-collapse-ports

   Keep every ``in`` port its own signal instead of an alias of its actual (to compare against the default).

.. option:: -g NAME=VALUE

   A generic of the top level, ``-gNAME=VALUE`` (the value is taken verbatim).

   May be repeated.

``yavs run``
------------

.. program:: yavs run

Elaborate and simulate a design.

.. code-block:: text

   yavs run [OPTIONS] --work=NAME:PATH TOP [+NAME[=VALUE]...]

Arguments:

* ``TOP``: ``ENTITY``, ``ENTITY.ARCH`` or ``CONFIGURATION`` in the work library.
* ``+NAME[=VALUE]`` (any number): Plusargs ``+NAME[=VALUE]`` after the top level, seen by VHPI plugins as tool arguments.

.. option:: --std=93|2008|2019

   Design revision of std/ieee; default: the newest standard in the design's closure.

.. option:: --work=NAME:PATH

   The library holding the top level, ``NAME:PATH``.

   Required.

.. option:: --map=NAME:PATH

   Another library, ``NAME:PATH`` (repeatable).

   May be repeated.

.. option:: --relaxed

   Turn some errors into warnings: a shared variable of a non-protected type, a pure function that calls an impure one, an ``others`` aggregate for an unconstrained target, a non-static actual of a signal parameter.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas (native PSL statements are always read).

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``: the same definitions as when the files were analyzed, because the sources are read again.

   May be repeated.

.. option:: -W FLAG

   Warning control: ``-Werror`` (every warning is an error), ``-Werror=CODE``, ``-Wno-CODE`` (do not print it), ``-WCODE`` (stays a warning under ``-Werror``).

   May be repeated.

.. option:: --no-cache

   Neither read nor write the compilation cache of the work library.

.. option:: --std-cache=DIR|off

   Directory of the std/ieee units shared by all libraries, or ``off`` (default: ``$YAVS_STD_CACHE``, else ``yavs/std`` in the user's cache directory).

.. option:: --no-collapse-ports

   Keep every ``in`` port its own signal instead of an alias of its actual (to compare against the default).

.. option:: -g NAME=VALUE

   A generic of the top level, ``-gNAME=VALUE`` (the value is taken verbatim).

   May be repeated.

.. option:: --exit-severity=note|warning|error|failure

   A report at or above this severity stops the simulation with exit 1. Without the option the default is ``failure``, and an ``error`` report is printed, the run continues and exits 1 at the end; given explicitly, a report below the level never fails the run.

.. option:: --ieee-warnings=on|off

   ``off`` suppresses the warnings raised inside ``ieee`` packages, such as ``NUMERIC_STD.TO_INTEGER: metavalue detected`` (default ``on``).

.. option:: --wave=FILE.fst

   Write an FST waveform file of the whole design (an unwritable path is E0801).

.. option:: --wave-include=GLOB

   Record only signals whose path (``top.u1.sig``) matches this glob (repeatable).

   May be repeated.

.. option:: --wave-exclude=GLOB

   Do not record signals or scopes whose path matches this glob (repeatable).

   May be repeated.

.. option:: --wave-max-array=N

   Leave out arrays and vectors with more elements than this.

   Default: ``65536``.

.. option:: --wave-variables

   With ``--wave``: also record the scalar variables of processes (sampled at the end of each time step); the include and exclude globs apply to ``top.proc.var``.

.. option:: --time-report

   Print the wall and CPU time of each phase (analyze, elaborate, init, simulate) to standard error when the simulation ends.

.. option:: --profile, --profile=FILE

   Sample where the simulation spends its time and print the busiest processes, instances and subprograms to standard error when the run ends; with FILE also write them as JSON.

.. option:: --stop-time=TIME

   Stop the simulation at this time and exit 0: a number and a unit, ``10ms`` or ``10 ns`` (a bare number is E0105).

.. option:: --hang-report, --hang-report=MAX

   Print what every waiting process waits for when the run ends at ``--stop-time`` without ``stop`` or ``finish``, or runs out of events with a process still waiting for a signal; at most MAX processes are listed per part (default 30). ``kill -USR1 PID`` prints the same report of a running simulation (Unix, always available).

.. option:: --max-deltas=N

   Most delta cycles at one simulated time before the run fails with E3002.

   Default: ``10000``.

.. option:: --resolution=fs|ps|ns|us|ms|sec

   Time resolution limit (``std.env.resolution_limit``, TIME values in reflection).

   Default: ``fs``.

.. option:: --threads=N

   Worker threads for process evaluation (at least 1). The output does not depend on it.

   Default: ``1``.

.. option:: --gc-threshold=SIZE

   Collect the heap of access values once this much was allocated since the last collection (and as much as that collection kept): plain bytes or a ``k``, ``m`` or ``g`` suffix (powers of 1024), at least 1. Default 1m.

.. option:: --shuffle, --shuffle=SEED

   Run the processes of a delta cycle in a random order, to find testbenches that depend on the order. SEED defaults to 1; the seed is printed at the start so that a failing run can be replayed with ``--shuffle=SEED``.

.. option:: --cov, --cov=FILE

   Count statement, branch and toggle coverage and write the run's coverage database to FILE (default ``yavs.ycov``).

.. option:: --cov-kind=KIND,...

   With ``--cov``: the kinds of coverage to count, comma separated.

   A list, values separated by ``,``. Default: ``statement,branch,toggle``.

   Values:

   * ``statement``: Each statement executed
   * ``branch``: Each arm of ``if``, ``case``, conditional and selected assignments, and loop bodies
   * ``toggle``: The 0 to 1 and 1 to 0 changes of every ``bit`` and ``std_ulogic`` scalar of signals and ports
   * ``condition``: Each input and value of conditions with two or more inputs
   * ``expression``: The same for the logic expression assigned to a signal or variable

.. option:: --cov-test=NAME

   With ``--cov``: the name of this run's test in the database (default: the enabled test case of a ``runner_cfg`` generic, else the top level with its ``-g`` generics).

.. option:: --cov-toggle-max=N

   With ``--cov``: leave signals with more ``bit``/``std_ulogic`` scalars than this out of toggle coverage.

   Default: ``65536``.

.. option:: --cov-exclude-translate-off

   With ``--cov``: also leave ``translate_off`` ... ``translate_on`` regions out of the coverage.

.. option:: --load=LIB

   Load a shared library for foreign subprograms (E0106 when it cannot be loaded, E0107 when a subprogram cannot be bound).

   May be repeated.

.. option:: --vhpi=LIB[:ENTRY]

   Load a VHPI plugin, ``LIB[:ENTRY]`` (E0901 when it cannot be loaded, E0902 when the entry point is missing).

   May be repeated.

.. option:: --tier=interp|llvm|auto|clift|clift-auto|auto3

   Execution tier: ``interp`` runs the interpreter alone; ``auto`` runs it and compiles the functions that become hot on a background thread; ``clift`` and ``llvm`` compile every function before time 0; ``clift-auto`` is ``auto`` with Cranelift; ``auto3`` has Cranelift compile warm functions and LLVM O2 those that stay hot. A tier needs its code generator in the build (error E0120 otherwise): this yavs has the tiers ``yavs --version`` names. Default: ``auto`` with a compiled tier, ``interp`` with ``--dap``, ``--interactive`` or ``--do``, and without one.

   Values:

   * ``interp``: The interpreter alone
   * ``llvm``: LLVM: every function before time 0 (the ``llvm`` feature)
   * ``auto``: The interpreter, and the functions that become hot compiled on a background thread
   * ``clift``: Cranelift: every function before time 0 (the ``clift`` feature)
   * ``clift-auto``: Cranelift: the functions that become hot, on a background thread
   * ``auto3``: Three tiers: the interpreter, Cranelift for warm functions, LLVM O2 for those that stay hot (needs the ``llvm`` and ``clift`` features)

.. option:: --tier-max-size=N

   With a compiled tier: functions of more than N IR instructions stay interpreted (compile time grows faster than their size).

   Default: ``50000``.

.. option:: --verbose

   Print notes on how the run is set up (functions a compiled tier leaves interpreted or compiles).

.. option:: --dap=PORT|stdio

   Run under a Debug Adapter Protocol client: on 127.0.0.1:PORT (0 picks a free port) or on standard input and output (``stdio``). A port that cannot be opened is E0701.

.. option:: --interactive

   Run from a ``yavs>`` prompt: stops at time 0, then takes console commands (``help`` lists them; run, step, examine, force, breakpoints, restart, source, quit).

   Cannot be combined with ``--dap``.

.. option:: --do=COMMANDS

   Run these console commands, separated by ``;``. Repeatable, in order. Without ``--interactive`` the run then exits with its status (1 at a command that fails).

   May be repeated. Cannot be combined with ``--dap``.

.. option:: --surfer, --surfer=HOST:PORT

   Show the waveform in Surfer (0.6.0 or later) while the run writes it: start ``surfer`` from PATH, or with ``=HOST:PORT`` connect to a running one whose WCP server listens there. Without ``--wave``, records ``<top>.fst`` in the directory that holds the work library.

.. option:: --surfer-refresh=DURATION

   How often the waveform is written out and Surfer reloads it (default ``1s``; ``500ms``, ``10s``); wins over ``YAVS_WAVE_FLUSH``.

.. option:: --source-map=COMPILED=ORIGINAL

   In a debug session (``--dap``, ``--interactive``, ``--do``), show the source file COMPILED as ORIGINAL and take breakpoints in ORIGINAL: for files compiled from a rewritten copy with the same lines. Repeatable.

   May be repeated.

Diagnostics named above: ``E0105``, ``E0106``, ``E0107``, ``E0120``, ``E0701``, ``E0801``, ``E0901``, ``E0902``, ``E3002``; ``yavs --explain CODE`` explains each.

``yavs cov merge``
------------------

.. program:: yavs cov merge

Add up the counts of several coverage databases into one.

.. code-block:: text

   yavs cov merge --output=FILE DB...

Arguments:

* ``DB`` (one or more): The coverage databases (``.ycov`` files written by ``yavs run --cov``).

.. option:: -o FILE, --output=FILE

   The merged database.

   Required.

``yavs cov report``
-------------------

.. program:: yavs cov report

Print a text summary of coverage databases (merged), and write other reports.

.. code-block:: text

   yavs cov report [OPTIONS] DB...

Arguments:

* ``DB`` (one or more): The coverage databases (``.ycov`` files written by ``yavs run --cov``).

.. option:: --cobertura=FILE.xml

   Also write a Cobertura XML report.

.. option:: --html=DIR

   Also write an HTML report (static files, sources read from the paths in the database) into DIR.

.. option:: --root=DIR

   Strip DIR from the front of the source paths the reports show (default: the current directory); a file outside DIR keeps its full path. The HTML report still reads sources from the paths in the database.

.. option:: --kind=LIST

   Report only these kinds of item (``statement``, ``branch``, ``toggle``, ``condition``, ``expression``, comma separated); ``--total`` and ``--fail-under`` then count only them.

   Accepted: ``statement``, ``branch``, ``toggle``, ``condition``, ``expression``.

   A list, values separated by ``,``.

.. option:: --total

   Print only the total coverage, as one percentage (``70.0%``, rounded down): items that ran over all items, after exclusions. A database with nothing to count is ``E0115``.

   Cannot be combined with ``--uncovered``, ``--tests``.

.. option:: --fail-under=PCT

   Exit with status 1 and ``E0114`` when the total coverage is below PCT (0 to 100, such as 80 or 92.5); the report is printed as usual. A PCT outside 0 to 100 is a usage error.

.. option:: --exclude=FILE

   Leave out the items an exclusion file lists (rows pasted from ``--uncovered``; a ``.yaml`` or ``.yml`` file is read as YAML).

.. option:: --uncovered, --uncovered=text|yaml

   Print only the items that never ran, as exclusion rows (``--uncovered=yaml``: as a YAML exclusion file).

.. option:: --tests

   Print every item that ran, as an exclusion row with the tests that hit it.

   Cannot be combined with ``--uncovered``.

.. option:: -W FLAG

   Warning control, as for ``analyze``: ``-Werror``, ``-Wno-CODE``, ...

   May be repeated.

Diagnostics named above: ``E0114``, ``E0115``; ``yavs --explain CODE`` explains each.

``yavs cov rank``
-----------------

.. program:: yavs cov rank

Rank the tests of merged databases: a minimal set reaching the same coverage, and what each test alone covers.

.. code-block:: text

   yavs cov rank [OPTIONS] DB...

Arguments:

* ``DB`` (one or more): The coverage databases (``.ycov`` files written by ``yavs run --cov``).

.. option:: --exclude=FILE

   Leave out the items an exclusion file lists.

.. option:: -W FLAG

   Warning control, as for ``analyze``: ``-Werror``, ``-Wno-CODE``, ...

   May be repeated.

``yavs lint``
-------------

.. program:: yavs lint

Report what can be proved wrong about a design before it is simulated.

.. code-block:: text

   yavs lint [OPTIONS] [FILE|DIR...]

Arguments:

* ``FILE|DIR`` (any number): The files and directories to check (``.vhd``, ``.vhdl``), read in this order. Default: every file recorded in the work library, which then needs ``--work``.

.. option:: -r, --recursive

   Look for files in the subdirectories of a directory too.

.. option:: --work=NAME:PATH

   The library the files belong to, ``NAME:PATH``. Without it they are read as library ``work``. Nothing is written into it; a file recorded in a library is read in the order of the library.

.. option:: --map=NAME:PATH

   Another library, ``NAME:PATH`` (repeatable).

   May be repeated.

.. option:: --std=93|2008|2019

   Language standard: the design revision, and the standard of a file no library records. Default: 2008 for such a file, and the newest standard among the files for the design.

.. option:: --config=FILE

   The configuration file. Default: ``yavs-lint.yaml`` in the working directory or an ancestor. Fails with E0134.

.. option:: --waivers=FILE

   The waiver file. Default: ``yavs-lint-waivers.yaml`` next to the configuration, or in the working directory or an ancestor. Fails with E0134.

.. option:: --list-rules

   Print every rule with its class and whether this configuration runs it, and exit.

.. option:: --explain=RULE

   Print what a rule reports and what it needs to report it, and exit.

.. option:: --json=FILE

   Write the findings as JSON. Fails with E0135.

.. option:: --sarif=FILE

   Write the findings as SARIF 2.1.0 for code scanning. Fails with E0135.

.. option:: --junit=FILE

   Write the findings as a JUnit file, one test case per source file. Fails with E0135.

.. option:: --quality-report=FILE

   Write the findings as a Code Climate report (GitLab code quality). Fails with E0135.

.. option:: --sonarqube=FILE

   Write the findings as SonarQube generic issues. Fails with E0135.

.. option:: --facts=FILE

   Write the design facts as JSON: state machines, clock domains, reset domains and crossings of each architecture. Fails with E0135.

.. option:: -W FLAG

   Finding control, by rule id: ``-Werror`` (a finding of severity warning is an error), ``-Werror=RULE``, ``-Wno-RULE`` (do not report it), ``-WRULE`` (stays a warning under ``-Werror``).

   May be repeated.

.. option:: --relaxed

   Turn some analysis errors into warnings, as ``yavs analyze --relaxed`` does.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas (native PSL statements are always read).

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``: the same definitions as when the files were analyzed, because the sources are read again.

   May be repeated.

.. option:: --no-cache

   Do not read the analysed-unit cache of the libraries.

.. option:: --std-cache=DIR|off

   Directory of the std/ieee units shared by all libraries, or ``off`` (default: ``$YAVS_STD_CACHE``, else ``yavs/std`` in the user's cache directory).

Diagnostics named above: ``E0134``, ``E0135``; ``yavs --explain CODE`` explains each.

``yavs cache clean``
--------------------

.. program:: yavs cache clean

Remove the cache entries of libraries.

.. code-block:: text

   yavs cache clean [OPTIONS] --work=NAME:PATH

.. option:: --work=NAME:PATH

   A library, ``NAME:PATH``; the cache of its directory is removed.

   Required. May be repeated.

.. option:: --std-cache=DIR|off

   Also remove the shared std/ieee units from DIR (default: ``$YAVS_STD_CACHE``, else the user's cache directory); ``off`` leaves them.

Environment variables
---------------------

None is needed, and options win over them. :doc:`environment` lists every one.

Exit status
-----------

.. list-table::
   :header-rows: 1
   :widths: 15 85

   * - Status
     - Meaning
   * - 0
     - Success. ``run``: the simulation ended without failure (no more events, ``stop`` or
       ``finish`` with 0, or ``--stop-time`` reached).
   * - 1
     - ``parse`` and ``analyze``: a file has errors. ``run``: a failed assertion or report at the
       exit severity, a runtime error, or ``stop(N)`` or ``finish(N)`` with N other than 0 (the
       status is N when N is from 1 to 125). ``cov``: a database or exclusion file could not be read or a report written, or the total is below ``--fail-under``. ``cache``: a file could not be written.
   * - 2
     - Usage error, or a library, analysis or elaboration error; also an unknown code for
       ``--explain``. A runtime error while the design is initialized, before time 0, is an
       elaboration error.
   * - 3
     - Internal error (``E9999``): please report it.
   * - 130
     - ``run``: stopped by Ctrl-C or SIGTERM.
   * - 141
     - ``run``: standard output was closed or could not be written.

Nothing runs after a failed step, and a status of 0 is never given after an error was printed.

Diagnostics
-----------

Errors and warnings go to standard error with a stable code: ``E`` for errors and ``W`` for
warnings. ``--message-format`` picks the form and ``yavs --explain CODE`` prints a longer
explanation. Codes by range: ``E0xxx`` syntax, command line, libraries, waveforms, coverage,
debugger and plugins; ``E1xxx`` semantic analysis; ``E2xxx`` elaboration; ``E3xxx`` runtime
errors; ``E9999`` an internal error. :doc:`error-codes` lists every code with its meaning, and
:doc:`troubleshooting` the common problems.

See also
--------

* :doc:`cli`: running simulations, by task.
* :doc:`vunit-yavs`: the options of the VUnit package.
* :doc:`environment`: environment variables.
* :doc:`error-codes`: every diagnostic code.
