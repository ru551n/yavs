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
     - Analyze VHDL files into a library
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
     - Print the version, whether a compiled tier is built in, and the copyright notice.

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

   Print the version and the copyright notice, and exit.

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

Analyze VHDL files into a library.

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

   Turn some errors into warnings: a shared variable of a non-protected type, a pure function that calls an impure one, an ``others`` aggregate for an unconstrained target, a non-static actual of a signal parameter, an operation a VHDL-1993 file sees declared both implicitly and explicitly, a PSL declaration after ``begin``.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas (native PSL statements are always read).

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``.

   May be repeated.

.. option:: -W FLAG

   Warning control: ``-Werror`` (every warning is an error), ``-Werror=CODE``, ``-Wno-CODE`` (do not print it), ``-WCODE`` (stays a warning under ``-Werror``).

   May be repeated.

.. option:: --coverage-directives=apply|ignore

   What to do with the ``-- yavs coverage exclude`` directives in the sources: ``apply`` (check them) or ``ignore`` (read none of them; a note says how many were left out).

   Default: ``apply``.

   Values:

   * ``apply``: Check the directives and waive what they name
   * ``ignore``: Read none of them

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

   Turn some errors into warnings: a shared variable of a non-protected type, a pure function that calls an impure one, an ``others`` aggregate for an unconstrained target, a non-static actual of a signal parameter, an operation a VHDL-1993 file sees declared both implicitly and explicitly, a PSL declaration after ``begin``.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas (native PSL statements are always read).

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``: the same definitions as when the files were analyzed, because the sources are read again.

   May be repeated.

.. option:: -W FLAG

   Warning control: ``-Werror`` (every warning is an error), ``-Werror=CODE``, ``-Wno-CODE`` (do not print it), ``-WCODE`` (stays a warning under ``-Werror``).

   May be repeated.

.. option:: --load=LIB[:args=LAYOUT]

   Load a shared library for foreign subprograms (E0106 when it cannot be loaded, E0107 when a subprogram cannot be bound). Also loaded for ``elab``, because a signal's initial value may call it. ``:args=LAYOUT`` says how its functions take their arguments: ``length`` (default) or ``fat-pointer`` (E0136 for any other).

   May be repeated.

.. option:: --no-cache

   Neither read nor write the compilation cache of the work library nor the analysed units of the libraries.

.. option:: --std-cache=DIR|off

   Directory of the std/ieee units shared by all libraries, or ``off`` (default: ``$YAVS_STD_CACHE``, else ``yavs/std`` in the user's cache directory).

.. option:: --coverage-directives=apply|ignore

   What to do with the ``-- yavs coverage exclude`` directives in the sources: ``apply`` (check them and waive the combinations they name) or ``ignore`` (read none of them; a note says how many were left out).

   Default: ``apply``.

   Values:

   * ``apply``: Check the directives and waive what they name
   * ``ignore``: Read none of them

.. option:: --no-collapse-ports

   Keep every ``in`` port its own signal instead of an alias of its actual (to compare against the default).

.. option:: -g NAME=VALUE

   A generic of the top level, ``-gNAME=VALUE`` (the value is taken verbatim).

   May be repeated.

Diagnostics named above: ``E0106``, ``E0107``, ``E0136``; ``yavs --explain CODE`` explains each.

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

   Turn some errors into warnings: a shared variable of a non-protected type, a pure function that calls an impure one, an ``others`` aggregate for an unconstrained target, a non-static actual of a signal parameter, an operation a VHDL-1993 file sees declared both implicitly and explicitly, a PSL declaration after ``begin``.

.. option:: --psl

   Read ``-- psl`` comments as PSL pragmas (native PSL statements are always read).

.. option:: -D NAME=VALUE, --define=NAME=VALUE

   Define a conditional analysis identifier, ``NAME=VALUE``: the same definitions as when the files were analyzed, because the sources are read again.

   May be repeated.

.. option:: -W FLAG

   Warning control: ``-Werror`` (every warning is an error), ``-Werror=CODE``, ``-Wno-CODE`` (do not print it), ``-WCODE`` (stays a warning under ``-Werror``).

   May be repeated.

.. option:: --load=LIB[:args=LAYOUT]

   Load a shared library for foreign subprograms (E0106 when it cannot be loaded, E0107 when a subprogram cannot be bound). Also loaded for ``elab``, because a signal's initial value may call it. ``:args=LAYOUT`` says how its functions take their arguments: ``length`` (default) or ``fat-pointer`` (E0136 for any other).

   May be repeated.

.. option:: --no-cache

   Neither read nor write the compilation cache of the work library nor the analysed units of the libraries.

.. option:: --std-cache=DIR|off

   Directory of the std/ieee units shared by all libraries, or ``off`` (default: ``$YAVS_STD_CACHE``, else ``yavs/std`` in the user's cache directory).

.. option:: --coverage-directives=apply|ignore

   What to do with the ``-- yavs coverage exclude`` directives in the sources: ``apply`` (check them and waive the combinations they name) or ``ignore`` (read none of them; a note says how many were left out).

   Default: ``apply``.

   Values:

   * ``apply``: Check the directives and waive what they name
   * ``ignore``: Read none of them

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

   With ``--wave``: also record the variables of processes and shared variables (sampled at the end of each time step); the include and exclude globs apply to ``top.proc.var``. A debug session (``--dap``, ``--interactive``, ``--do``) that records a wave does this on its own.

.. option:: --time-report

   Print the wall and CPU time of each phase (analyze, elaborate, init, simulate) to standard error when the simulation ends.

.. option:: --profile, --profile=FILE

   Sample where the simulation spends its time and print the busiest processes, instances and subprograms to standard error when the run ends; with FILE also write them as JSON.

.. option:: --activity, --activity=FILE

   Count process activations and signal events and print the busiest to standard error when the run ends; with FILE also write them as JSON. Costs run time only when given.

.. option:: --assert-summary

   Print the counts of assertion failures and reports by severity and source place to standard error when the run ends; with ``--message-format=json``, one JSON object.

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

.. option:: --threads=N|auto

   Threads for process evaluation: a number of at least 1, or ``auto``, which decides while the run goes whether more threads make it faster and uses them only then (up to the CPUs the run may use, at most 8). The output does not depend on it.

   Default: ``1``.

.. option:: --gc-threshold=SIZE

   Collect the heap of access values once this much was allocated since the last collection (and as much as that collection kept): plain bytes or a ``k``, ``m`` or ``g`` suffix (powers of 1024), at least 1. Default 1m.

.. option:: --shuffle, --shuffle=SEED

   Run the processes of a delta cycle in a random order, to find testbenches that depend on the order. SEED defaults to 1; the seed is printed at the start so that a failing run can be replayed with ``--shuffle=SEED``.

.. option:: --cov, --cov=LETTERS

   Count coverage and write the run's coverage database (``--cov-file``). LETTERS selects the kinds, in any order: ``s`` statement, ``b`` branch, ``c`` condition, ``e`` expression, ``t`` toggle, ``f`` fsm, ``a`` assert, ``m`` combination; for example ``--cov=sbt``. No kind is on by default.

.. option:: --cov-kind=KIND,...

   Count coverage of these kinds, comma separated (``statement``, ``branch``, ``toggle``, ...): the long form of ``--cov=LETTERS``, not with it.

   A list, values separated by ``,``.

   Values:

   * ``statement``: Each statement executed
   * ``branch``: Each arm of ``if``, ``case``, conditional and selected assignments, and loop bodies
   * ``toggle``: The 0 to 1 and 1 to 0 changes of every ``bit`` and ``std_ulogic`` scalar of signals and ports
   * ``condition``: Each input and value of conditions with two or more inputs, ``wait until`` included
   * ``assert``: The same for the conditions of ``assert``. A row that fails the assertion is hit only by a failing run, so these rows are asked for apart; they report as conditions
   * ``expression``: The same for the logic expression assigned to a signal or variable
   * ``combination``: Each combination of the inputs of conditions and logic expressions that can occur (an input a short-circuit operator skips counts as not evaluated, not as false)
   * ``fsm``: Each state a state machine reached and each transition it took, per instance

.. option:: --cov-file=FILE

   With coverage on (``--cov`` or ``--cov-kind``): the coverage database to write (default ``yavs.ycov``).

.. option:: --cov-test=NAME

   With coverage on: the name of this run's test in the database (default: the enabled test case of a ``runner_cfg`` generic, else the top level with its ``-g`` generics).

.. option:: --cov-toggle-max=N

   With coverage on: leave signals with more ``bit``/``std_ulogic`` scalars than this out of toggle coverage.

   Default: ``65536``.

.. option:: --cov-exclude-translate-off

   With coverage on: also leave ``translate_off`` ... ``translate_on`` regions out of the coverage.

.. option:: --cov-embed-sources

   With coverage on: store the text of the source files in the database, so ``yavs cov report --html`` needs no checkout. The source text then travels with the database.

.. option:: --vhpi=LIB[:ENTRY]

   Load a VHPI plugin, ``LIB[:ENTRY]`` (E0901 when it cannot be loaded, E0902 when the entry point is missing).

   May be repeated.

.. option:: --tier=interp|auto|clift|clift-auto

   Execution tier: ``interp`` runs the interpreter alone; ``auto`` runs it and compiles the functions that become hot (see ``--tier-compile``); ``clift`` compiles every function before time 0; ``clift-auto`` is another name for ``auto``. A tier needs its code generator in the build (error E0120 otherwise): this yavs has the tiers ``yavs --version`` names. Default: ``auto`` with a compiled tier, ``interp`` with ``--dap``, ``--interactive`` or ``--do``, and without one.

   Values:

   * ``interp``: The interpreter alone
   * ``auto``: The interpreter, and the functions that become hot compiled during the run
   * ``clift``: Cranelift: every function before time 0 (the ``clift`` feature)
   * ``clift-auto``: Cranelift: the functions that become hot, during the run

.. option:: --tier-max-size=N

   With a compiled tier: functions of more than N IR instructions stay interpreted (compile time grows faster than their size).

   Default: ``50000``.

.. option:: --tier-compile=wait|background

   With ``--tier=auto``: where the functions that become hot are compiled. ``wait``: the run waits while one compiles instead of running it interpreted meanwhile; a tier-up thread still prepares functions alongside the run, so a run can use somewhat more than ``--threads`` cores for short periods; ``background``: on a thread of its own, at the lowest priority of the system, alongside the run. ``--tier=interp`` starts no tier-up thread.

   Default: ``wait``.

   Values:

   * ``wait``: The run waits while a function compiles
   * ``background``: A thread of its own compiles alongside the run, at the lowest priority

.. option:: --verbose

   Print notes on how the run is set up (functions a compiled tier leaves interpreted or compiles).

.. option:: --dap=PORT|stdio

   Run under a Debug Adapter Protocol client: on 127.0.0.1:PORT (0 picks a free port) or on standard input and output (``stdio``). A port that cannot be opened is E0701.

.. option:: --exit-on-stdin-close

   End the run (exit status 130) when standard input is closed or ends, for a parent that holds a pipe to it and may die without a word (SIGKILL, a crash): the run does not outlive it, even while it waits for a debugger. Takes standard input for itself (the design and every library see the null device), so not with ``--interactive``; with ``--dap=stdio`` the protocol stream plays this part. Waveform and coverage files are not finished.

   Cannot be combined with ``--interactive``.

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

Diagnostics named above: ``E0105``, ``E0106``, ``E0107``, ``E0120``, ``E0136``, ``E0701``, ``E0801``, ``E0901``, ``E0902``, ``E3002``; ``yavs --explain CODE`` explains each.

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

.. option:: --json=FILE.json

   Also write a JSON report with every kind and test (schema in the coverage guide).

.. option:: --lcov=FILE.info

   Also write an LCOV tracefile: lines and branches only.

.. option:: --sonarqube=FILE.xml

   Also write a SonarQube generic coverage report: lines and branches only.

.. option:: --html=DIR

   Also write an HTML report (static files; sources embedded with ``--cov-embed-sources``, else read from the paths in the database) into DIR.

.. option:: --root=DIR

   Strip DIR from the front of the source paths the reports show (default: the current directory); a file outside DIR keeps its full path. The HTML report still reads sources from the paths in the database.

.. option:: --kind=LIST

   Report only these kinds of item (``statement``, ``branch``, ``toggle``, ``condition``, ``expression``, ``combination``, comma separated); ``--total`` and ``--fail-under`` then count only them.

   Accepted: ``statement``, ``branch``, ``toggle``, ``condition``, ``expression``, ``combination``, ``fsm``.

   A list, values separated by ``,``.

.. option:: --total

   Print only the total coverage, as one percentage (``70.0%``, rounded down): items that ran over all items, after exclusions. A database with nothing to count is ``E0115``.

   Cannot be combined with ``--uncovered``, ``--tests``.

.. option:: --fail-under=PCT|KIND:PCT,...

   Exit with status 1 and ``E0114`` when the total coverage is below PCT (0 to 100, such as 80 or 92.5), or a kind is below its own (``statement:90,branch:80``); the report is printed as usual. A PCT outside 0 to 100 or an unknown kind is a usage error.

.. option:: --below=PCT

   Limit the table of instances and packages to those below PCT, worst first, with a line counting the rest. Counts what the report counts (``--kind``, exclusions), rounded down. Totals and ``--fail-under`` are not affected.

   Cannot be combined with ``--uncovered``, ``--tests``, ``--total``, ``--exclusions``.

.. option:: --detailed

   Also show, per instance after ``--below``, ``--kind`` and the exclusions, a table per kind with every item, and for what never ran its source, what is missing and a tip. ``--uncovered`` is the row format for exclusion files; ``--exclusions`` the full audit.

   Cannot be combined with ``--uncovered``, ``--tests``, ``--total``, ``--exclusions``.

.. option:: --exclude=FILE

   Leave out the items an exclusion file lists (rows pasted from ``--uncovered``; a ``.yaml`` or ``.yml`` file is read as YAML). Repeatable: the rules of all files add up. A file that cannot be read or has a bad row is ``E0112``.

   May be repeated.

.. option:: --config=FILE

   Read options from a YAML file: ``exclude`` (exclusion files, added before those of ``--exclude``), ``root`` (both relative to the file) and ``kind``; ``--root`` and ``--kind`` win. Only this option reads such a file: nothing is found on its own. A file that cannot be read or has an unknown key is ``E0138``.

.. option:: --exclusions, --exclusions=text|json

   Print the exclusion report instead of the summary: every rule that takes requirements out of the totals (``-- yavs coverage exclude`` directives and the rows of ``--exclude``) with its origin, reason, what it matched, how often that ran, and the totals before and after (``--exclusions=json``: versioned JSON). The HTML report gets the same page.

   Cannot be combined with ``--uncovered``, ``--tests``, ``--total``.

.. option:: --uncovered, --uncovered=text|yaml

   Print only the items that never ran, as exclusion rows (``--uncovered=yaml``: as a YAML exclusion file).

.. option:: --tests

   Print every item that ran, as an exclusion row with the tests that hit it.

   Cannot be combined with ``--uncovered``.

.. option:: -W FLAG

   Warning control, as for ``analyze``: ``-Werror``, ``-Wno-CODE``, ...

   May be repeated.

Diagnostics named above: ``E0112``, ``E0114``, ``E0115``, ``E0138``; ``yavs --explain CODE`` explains each.

``yavs cov rank``
-----------------

.. program:: yavs cov rank

Rank the tests of merged databases: a minimal set reaching the same coverage, and what each test alone covers.

.. code-block:: text

   yavs cov rank [OPTIONS] DB...

Arguments:

* ``DB`` (one or more): The coverage databases (``.ycov`` files written by ``yavs run --cov``).

.. option:: --exclude=FILE

   Leave out the items an exclusion file lists. Repeatable: the rules of all files add up. A file that cannot be read or has a bad row is ``E0112``.

   May be repeated.

.. option:: --config=FILE

   Read the ``exclude`` key of a YAML file, as ``cov report --config`` does (its other keys are checked and have no effect here). A file that cannot be read or has an unknown key is ``E0138``.

.. option:: -W FLAG

   Warning control, as for ``analyze``: ``-Werror``, ``-Wno-CODE``, ...

   May be repeated.

Diagnostics named above: ``E0112``, ``E0138``; ``yavs --explain CODE`` explains each.

``yavs lint``
-------------

.. program:: yavs lint

Report what can be proved wrong about a design before it is simulated.

.. code-block:: text

   yavs lint [OPTIONS] [FILE|DIR...]

Arguments:

* ``FILE|DIR`` (any number): The files and directories to check (``.vhd``, ``.vhdl``), read in this order, before those of ``file_list`` in the configuration. With neither: every file recorded in the work library, which then needs ``--work``.

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

   The configuration file. Default: ``yavs-lint.yaml`` in the working directory or an ancestor. A bad value, an unknown key or an unknown rule fails with E0134, all of them listed.

.. option:: --waivers=FILE

   The waiver file. Default: ``yavs-lint-waivers.yaml`` next to the configuration, or in the working directory or an ancestor. A bad value, an unknown key or an unknown rule fails with E0134, all of them listed.

.. option:: --no-file-list

   Do not check the files of ``file_list`` in the configuration.

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
       status is N when N is from 1 to 125). ``cov``: a database or exclusion file could not be read or a report written, or the total is below ``--fail-under``. ``cache``: a file could not be written. Any command of an x86_64 release archive (not a ``-compat`` one) on a CPU without x86-64-v3: it prints the missing features and the way out, and nothing else runs.
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
