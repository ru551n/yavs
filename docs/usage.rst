Command line
============

yavs works like the other command line simulators: analyze the files into libraries, then
elaborate and run a top level. ``yavs --help`` and ``yavs <command> --help`` list every option.

Libraries
---------

A library is a directory, given as ``NAME:PATH``. ``--work`` is the library a command works in,
``--map`` adds another one (repeatable). A missing or empty directory is an empty library;
``analyze`` creates it. ``std`` and ``ieee`` are always available and cannot be mapped.

A library records which file holds which unit, and ``elab`` and ``run`` analyze the units the top
level needs. They keep the result in a cache next to the library, see `Compilation cache`_, so
there is no compiled library to keep up to date and nothing to rebuild by hand.

Analyze
-------

.. code-block:: bash

    yavs analyze --std=2019 --work=lib:out/lib pkg.vhd tb.vhd

The files are analyzed in order; the first failing file stops the batch. Pass the consecutive files of one library and standard in one call: the call shares its parsed dependencies and the library index,
which makes the cold analysis of a whole library 26 to 50 percent cheaper in user instructions than one call per file, and a failing call is repeated file by file when each file's own result is needed. ``--std`` is the
standard of these files (``2008`` by default). ``-D NAME=VALUE`` defines a conditional analysis
identifier. An argument ``@FILE`` is replaced by the arguments in FILE, one per line.

Elaborate and run
-----------------

.. code-block:: bash

    yavs run --work=lib:out/lib -gWIDTH=8 --stop-time=10ms tb

The top level is ``ENTITY``, ``ENTITY.ARCH`` or a configuration. The design is analyzed against
one revision of ``std`` and ``ieee``, the newest standard of its files, unless ``--std`` says
otherwise.

* ``--exit-severity=failure`` (the default): a report of that severity or higher stops the
  simulation with exit status 1. An ``error`` report below it is printed, the run continues,
  and the exit status is 1 at the end, unless ``ClearVhdlAssert`` (VHDL-2019 ``std.env``)
  cleared the assert counts since. When the option is given, even as ``--exit-severity=failure``,
  only reports at or above that level affect the exit status.
* ``--time-report`` prints the wall and CPU time of each phase (``analyze``, ``elaborate``,
  ``init``, ``simulate``) and the total to standard error when the simulation ends, for example
  ``time-report: simulate wall=1.204 cpu=1.19``. CPU time is for the whole process, in 10 ms
  steps, and is shown as ``n/a`` on platforms that do not report it.
* ``--profile[=FILE]`` finds where a long simulation spends its time. A sampler takes 1000
  samples a second and, when the run ends, prints the busiest processes (instance path, source
  file and line), instances, subprograms and source lines to standard error, for example
  ``profile:   93.2     2280  :top:dut:hot (dut.vhd:15)``. Give a file name to also get all rows
  as JSON. Press Ctrl-C to stop a long run: the profile of the time so far is printed (and
  written) and the exit status is 130. The output of the design is unchanged and the run is slowed by by about 2 to 3 percent.
  Source lines are known for interpreted code only (``--tier=interp``, or code that
  ``--tier=auto`` has not compiled yet); a small subprogram is inlined into its caller and
  counts as the caller in the subprogram table. With ``--threads N`` a sample counts every
  thread that runs a process, so shares can add up to more than 100 percent. ``--profile``
  cannot be combined with ``--dap`` (``E0125``).
* ``--threads N`` evaluates processes on N threads (default 1). The output is identical for any
  N. ``--shuffle[=SEED]`` randomizes the process order to find testbenches that depend on it.
* ``--max-deltas N`` sets how many delta cycles may run at one simulated time (default 10000)
  before the run fails with ``E3002``. The error names the processes and signals of the loop and
  how they wake each other; raise the limit only for a design that needs more deltas.
* ``--hang-report[=MAX]`` explains a simulation that did not finish. When the run ends at
  ``--stop-time`` without ``stop`` or ``finish``, or runs out of events while a process still waits
  for a signal, it prints warning ``W3101``: how many processes wait for a timeout, for signals that
  can still change, or can never run again, what those processes wait for, and for a signal nothing
  can change its value, last event and drivers ("it has no driver" is the usual answer). A clock
  that stopped toggling while a process waits for its edge is named. At most MAX processes are
  listed per part (default 30). Without the option nothing is printed at those ends. On Unix,
  ``kill -USR1 PID`` prints the same report of a running simulation, which then goes on. A
  process with no wait statement and no sensitivity list never suspends, so time cannot advance;
  ``W3102`` warns about it before time 0, and its activation ends with ``E3009`` after 10^9 jumps back.
* ``--shuffle[=SEED]`` prints its seed at the start of the run and repeats it in the diagnostics
  of the run; ``--shuffle=SEED`` replays the same order.
* ``--gc-threshold SIZE`` sets how much memory access values may allocate (plain bytes, or a
  ``k``, ``m`` or ``g`` suffix; at least 1) before the heap is collected, during elaboration
  and in the simulation. A collection also waits until as much was allocated as the last one
  kept, so its cost stays proportional to the allocation. A larger value collects less often
  and uses more memory on designs that allocate heavily; the default is ``1m``. The output is
  the same for any value. There is no way to switch collection off.
* ``--tier=auto`` compiles only the functions that become hot, on a background thread while the
  simulation runs in the interpreter; ``--tier=llvm`` compiles every function with LLVM before
  time 0 and runs that code; ``--tier=interp`` runs the interpreter alone. The output is the same
  with every tier. ``auto`` is the default in a yavs built with the ``llvm`` feature (LLVM 21),
  except under ``--dap``, which runs in the interpreter unless you ask for a tier;
  a yavs built without the feature always defaults to ``--tier=interp``, and ``--tier=auto`` or
  ``--tier=llvm`` on it is an error. Functions of
  more than ``--tier-max-size`` IR instructions (default 50000) stay interpreted. ``--verbose``
  prints what the LLVM tier compiles and what it leaves to the interpreter.
  ``--tier=clift`` and ``--tier=clift-auto`` do the same with Cranelift instead of LLVM, in a yavs built
  with the ``clift`` feature; that needs no LLVM install.
  ``--tier=auto3`` combines them (Cranelift for warm functions, LLVM O2 for those that stay hot) in a yavs built with both features.
* ``--resolution=ps`` sets the time resolution limit (``fs``, ``ps``, ``ns``, ``us``, ``ms`` or
  ``sec``; default ``fs``). It is what ``std.env.resolution_limit`` returns and the unit of TIME
  values in ``std.reflection`` mirrors. A TIME literal with a unit below the limit anywhere in
  the elaborated design (``1 ps`` under ``--resolution=ns``), in a package it uses or in code that
  never runs, is an error (``E2120``, LRM 5.2.4.2) that lists the locations; ``0 ps`` is allowed,
  and so is ``0.4 ns``, because the rule is about the unit and not the value.
* An ``in`` port whose actual is a signal, or an element or slice of one, shares its actual's storage: no separate
  signal and no copy each delta cycle, with the same output, waveforms and names.
  ``--no-collapse-ports`` keeps every port its own signal. Ports that are forced, and all ports
  under ``--dap`` or ``--vhpi``, always are.
* ``--wave out.fst`` records waveforms, see `Waveforms`_.
* ``--cov[=FILE]`` counts code and toggle coverage, see `Code coverage`_.

Compilation cache
-----------------

``elab`` and ``run`` analyze the files of the design and keep the result in the work library, in
``yavs-cache/`` beside the manifest. The first run of a design only notes it (a design that runs
once pays nothing for an image nobody reads); the second stores the result, and every later run of
the same design reads it back, which is much faster for a short simulation (a test case of a large
verification library mostly spends its time analyzing). Nothing changes in the results: the output,
warnings and exit status are the same with and without it, at any ``--threads``.

The first files of a design are shared between designs. The test cases of one library start with
the same packages; the analysis of those files is kept once, in the cache of the library of the
last of them, and a design that starts with the same files and options continues from it. So the
first run of a new test case of a library that other test cases already used analyzes only its
own files.

An entry is used only when it fits the run exactly: the same yavs, the same options (``--std``,
``--relaxed``, ``--psl``, ``--encoding``) and the same files with the same contents. Edit a file,
or a file it depends on, and the next run analyzes again and stores a new entry. A library
directory that cannot be written is not cached into and works as before; entries that are
damaged or from another yavs are ignored.

The lowered form of the packages a design uses (the ones that hold signals, shared variables or
values computed when the design starts) is kept in the same directory, so a short run skips that
work too. The test cases of one library share this as well: the library's lowered code, the state
of its packages after they start and the optimised code of the functions earlier test cases called
are kept once next to the library (about 10 MB for a large verification library; a library
directory keeps at most 512 MiB of entries), made when a few different test cases (four) have asked for them, so the
first run of a new test case does not make them again.

With ``--tier=llvm`` or ``--tier=auto`` the machine code of the compiled tier is kept there as
well (``yavs-cache/obj/``). The next run of the design, and the other test cases of a testbench,
link that code instead of compiling it again; ``--tier=auto`` uses the stored code of a hot
computation loop as soon as the loop is hot, without waiting until compiling it would pay off. The
code is the same that compiling makes, so the results are too.

* ``--no-cache`` (``elab``, ``run``) neither reads nor writes the cache.
* ``yavs cache clean --work=lib:out/lib [--map=NAME:PATH ...]`` removes the entries of those
  libraries. Deleting ``yavs-cache/`` by hand does the same.

Waveforms
---------

.. code-block:: bash

    yavs run --work=lib:out/lib --wave=tb.fst --wave-exclude='tb.u_ddr.**' tb

``--wave FILE.fst`` records the whole design as an FST file that waveform viewers read:

* The hierarchy follows the elaborated design: instances (shown with their ``entity(arch)``),
  blocks, generate scopes (``g(3)`` for each for-generate index, ``g.alt1`` for the chosen
  alternative of an if or case generate with a label) and package
  signals (``work.pkg.sig``). Names keep the spelling of the source.
* Every signal and port is recorded, ports with their direction. ``std_logic`` and ``bit``
  values and their vectors show as characters (``U``, ``X``, ``Z``...) with the vector bounds
  and direction kept; other enumerations show their literal names; integers, ``time`` and
  ``real`` show as numbers. An integer carries the name of its subtype (``natural``, or your
  ``small_t``) and the width its range needs: ``natural range 0 to 255`` is 8 bits, a range with
  negative values is signed (``-8 to 7`` is 4 bits), and a range that depends on a generic is
  sized after elaboration. Records are scopes with one entry per field, and arrays of anything
  other than ``std_logic`` or ``bit`` are scopes with one entry per element (``mem(2)``,
  ``grid(1,2)``). Variables are only recorded with ``--wave-variables``.
* A port that is connected to a signal, and the signal itself, are one value: the file holds
  its data once and each name (the signal and the port at every level of the hierarchy) points
  to it, so a viewer shows all of them with the same changes and the file does not grow with
  the depth of the hierarchy.
* FST has no delta cycles: several deltas at one time step give one change with the final
  value. The file is the same for any ``--threads``.
* ``--wave-include GLOB`` and ``--wave-exclude GLOB`` (repeatable) choose what is recorded by
  path, such as ``tb.u1.count``: matching ignores case, ``*`` matches within one level, ``**``
  across levels and ``?`` one character, and a pattern that matches a scope covers everything
  below it. Signals left out cost nothing while simulating.
* ``--wave-variables`` also records variables: those of processes, in a scope named after the
  process (``tb.main.count``), and shared variables, in the region or package that declares
  them (``tb.u1.acc``, ``work.pkg.total``). They are shown like signals: records and arrays as
  scopes, ``std_logic`` and ``bit`` vectors as vectors, and an integer with the width of its
  range after elaboration (``integer range 0 to DEPTH - 1`` is 4 bits when ``DEPTH`` is 16). The
  include and exclude globs and ``--wave-max-array`` apply to them. A variable is sampled at the
  end of each time step, so the file shows its value after all the deltas of that step and is
  the same for any ``--threads``; a process that ends keeps the values it had when it last
  suspended. Without the option, variables are not recorded and cost nothing. Not recorded:
  access and file variables, shared variables of a protected type, and a variable whose bounds
  are known only when the simulation runs (warning ``W0802``).
* Arrays and vectors with more elements than ``--wave-max-array`` (default 65536) are left out
  with warning ``W0802``.
* The file is written in blocks of 250000 changes and finished at the end of the simulation, at
  ``--stop-time`` and on Ctrl-C or SIGTERM: the first Ctrl-C ends the run after the current
  cycle and exits with status 1, a second one exits at once (the file is then readable up to
  its last block). The bytes depend only on the recorded changes, so two runs of the same design
  write identical files. To make a run that is killed (SIGKILL, a crash) leave more of the
  file, set ``YAVS_WAVE_FLUSH`` to a duration such as ``5s`` or ``500ms``: a block then also ends
  after that much run time, and the bytes depend on the speed of the run. ``YAVS_WAVE_FLUSH=never``
  is the default; any other value is error ``E0801``. A path that cannot be written is error
  ``E0801``.

Code coverage
-------------

.. code-block:: bash

    yavs run --work=lib:out/lib --cov=test_a.ycov tb -gtest=a
    yavs run --work=lib:out/lib --cov=test_b.ycov tb -gtest=b
    yavs cov merge test_a.ycov test_b.ycov -o all.ycov
    yavs cov report all.ycov --cobertura coverage.xml

``--cov[=FILE]`` counts how often every statement and every branch of the design ran and how
often every ``std_logic`` and ``bit`` signal toggled, and writes the counts to the coverage
database ``FILE`` (default ``yavs.ycov``) at the end of the run, whatever its outcome. Without
``--cov`` there are no counters and no cost. ``--cov-kind=LIST`` counts only some kinds
(``statement``, ``branch``, ``toggle``, ``condition``, ``expression``, comma separated; default
``statement,branch,toggle``; ``condition`` and ``expression`` cost extra instructions in
every evaluation of a multi-input condition, so they are asked for). Coverage never changes a
count by skipping work: assignments of the value a signal already holds queue no transaction
(as without ``--cov``), and a clocked process whose inputs did not change is left suspended
only when no statement, branch, condition or expression counter is on, so toggle coverage alone
runs nearly as fast as a plain run plus the toggle database.

* **Statements**: every sequential statement and every concurrent statement (each time it
  runs). Declarations are not counted.
* **Branches**: each arm of an ``if``/``elsif``/``else`` (an ``if`` without ``else`` has an arm
  for "no condition held"), each ``case`` alternative, each choice of a conditional
  (``when ... else``) or selected (``with ... select``) assignment, sequential or concurrent
  (one without a final ``else`` has the "no condition held" arm too), and the body of a ``for``
  or ``while`` loop (counted per iteration).
* **Toggles**: for every scalar ``std_ulogic``/``std_logic`` or ``bit`` of every signal and
  port (each element of a vector, each field of a record, ``mem(2)(7)``), how often it went
  from 0 to 1 and from 1 to 0. ``'L'`` counts as 0 and ``'H'`` as 1; every other value
  (``'U'``, ``'X'``, ``'Z'``, ``'W'``, ``'-'``) is neither, so a change through it is no toggle:
  ``'0'`` to ``'Z'`` to ``'1'`` counts nothing, and neither does the first ``'U'`` to ``'0'``.
  A scalar is covered when it toggled both ways. Signals are listed under the path of the
  scope that declares them (``:tb:u_dut``, ``:tb:g(3)``; package signals under
  ``library.package``) with their element name (``data(3)``, ``bus.valid``). Signals with more
  scalars than ``--cov-toggle-max`` (default 65536), such as large memories, are left out with
  warning ``W0115``. Toggle counting only runs with toggle coverage on.
* **Conditions and expressions** (focused expression coverage): for every condition with two or
  more inputs, one row per input and value. A *condition* is the boolean of an ``if``/``elsif``,
  a conditional (``when ... else``) assignment, a ``while``, an ``exit when`` or a ``next when``;
  an *expression* is the value of a signal or variable assignment. Both are trees of ``and``,
  ``or``, ``nand``, ``nor``, ``xor``, ``xnor`` and ``not`` on ``boolean``, ``bit`` or
  ``std_logic`` values; everything else in them (a comparison, a name, a function call,
  ``rising_edge(clk)``) is one input. Input ``X`` at value ``v`` is covered when, in some
  evaluation, flipping ``X`` alone would change the result while the other inputs keep their
  values. In ``if a and (b or c)`` with ``a`` true, ``b`` false and ``c`` true the
  condition is true, and flipping ``a`` or ``c`` alone makes it false, but flipping ``b`` does
  not: that evaluation covers ``a = 1`` and ``c = 1``, not ``b = 0``.
  An input that is constant after elaboration (a generic, a constant, a literal) is evaluated
  but has no rows, and a condition with fewer than two other inputs has none at all.
  An input that was not evaluated because the operator short-circuits (``and``/``or`` on
  ``boolean`` and ``bit`` skip the right operand when the left decides) is masked: it can take
  any value, so ``a and b`` with ``a`` false covers ``a = 0``. Counting never changes the order
  of evaluation or evaluates anything extra, so a function call used as an input runs as often as
  without coverage. For ``std_logic``, ``'0'`` and ``'L'`` are 0 and ``'1'`` and ``'H'`` are 1;
  an evaluation in which an input has any other value (``'U'``, ``'X'``, ``'Z'``, ``'W'``,
  ``'-'``) counts no row. Not measured: conditions of ``assert`` and ``wait until``, vector
  operands, logic expressions nested inside other expressions, and trees of more than 31 inputs.
* Code of an architecture is counted per instance and shown under its path (``:tb:u_dut``),
  blocks and generate iterations included. Code of a package is counted once under
  ``library.package``. The ``std`` and ``ieee`` libraries are not counted.
* Items are identified by their path and the text of their statement, not by its position in
  the file, so a database stays mergeable after edits elsewhere in the file. Counts are the same
  for any ``--threads``.
* **Clocked processes count only their edges.** A process that does nothing unless an edge
  happens (``if rising_edge(clk) then ... end if``, ``falling_edge``, ``clk'event and clk = '1'``,
  ``wait until rising_edge(clk)``, ``wait until clk = '1'``, also with an asynchronous reset) is
  resumed only at that edge, with or without ``--cov``. Its edge test therefore counts one run at
  initialisation and one per edge, not one per event on the clock, and the arm "no condition
  held" of a test that is only an edge shows one run (the initialisation), not the falling
  edges. The statements inside the ``if`` count exactly the same as when the process also ran
  at the other edge.

``yavs cov merge A.ycov B.ycov ... -o OUT.ycov`` adds up the counts of several runs: write one
database per run (one ``--cov=FILE`` each) and merge them. ``yavs cov report DB ...`` merges its
inputs, prints statement, branch and toggle totals, one line per instance or package, and every
item that never ran with its file and line (for a toggle, the signal's declaration);
``--cobertura FILE.xml`` also writes a Cobertura XML report (one entry per source line: the hits
of its most executed statement, and how many of its branches and condition rows ran) for CI
services that show coverage. Cobertura has no notion of toggles, so they are only in the text report. A database that cannot be read
is error ``E0111``, a file that cannot be written ``E0110``.

``--html DIR`` writes an HTML report into ``DIR``: static files with inline styles and one small
inline script and no external resources, so ``index.html`` opens straight from disk, in a light or
dark theme as the browser asks. The index has the totals, one line per instance or package with
its statement, branch and toggle percentages, and a link per source file. A file page shows the
source with the hit count of each line: missed lines in red, lines that ran in some instances
and not in others in purple with a ``1/2`` marker (instances it ran in out of all), and lines
with a missed branch arm or toggle in yellow; a legend explains the colours. Click a line, or
focus it with the keyboard and press Enter or Space, to open a popup with its items (statement, branch arm, toggle per
signal and direction) per instance with the count and, when the database has test names, the
tests that hit it. The popup opens only on a click, never on hover; clicking another line moves
it, clicking outside or pressing Escape closes it, and its text can be selected. It needs the
inline script: with scripts disabled nothing opens. A condition or expression has a table of its
rows in the popup (``row``, ``input``, the ``result`` the input decides alone, ``?`` when an
``xor`` is in the way, and the ``hit`` count). A missed item
shows its ``--uncovered`` row and its ``--uncovered=yaml`` entry, each with a copy button, ready
for an exclusion file.
Each file also has ``src-N-missed.html`` (linked from the index and the file page), the same page
reduced to the lines with a missed or excluded item, to find the gaps in a big design. Items
taken out by ``--exclude`` stay visible, greyed, with the reason of their row. Source paths are
shown relative to the current directory; ``--root DIR`` strips another prefix instead (a path
without it is shown as it is). The sources are read at report time from the paths in the
database, so run the command where they resolve; a source that cannot be read is error ``E0113``
and nothing is written. The same database always gives the same bytes.

Excluding code from coverage
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

In the source, ``-- coverage off`` (optionally ``-- coverage off: reason``) starts a region
whose statements are left out of the database, ``-- coverage on`` ends it, and ``-- coverage
off-line`` on a line leaves out the statements starting there (``-- yavs coverage ...`` is the
same). A region that is never closed excludes the rest of the file (``W0112``).
``--cov-exclude-translate-off`` also leaves ``translate_off`` regions out.

Outside the source: ``yavs cov report --uncovered DB`` prints one row per item that never
ran, with the line number and the statement as a comment:

.. code-block:: text

    statement :tb:dut:u_fsm  4d2f0a91c3be.0  src/fsm.vhd:61   # state <= idle;
    branch    :tb:dut:u_fsm  c41e09a7b3d2.0.3  src/fsm.vhd:88   # arm 3: case state is
    toggle :tb:dut data(7) fall   # never 1->0 (src/dut.vhd:12)
    condition :tb:dut 5e5f6d18d9f4.0 3  src/dut.vhd:37   # b is 1 (result 1) in a and (b or c)

Copy the rows you want to exclude into a file, add a reason after ``#`` if you like, and give it
to the report:

.. code-block:: text

    # exclusions.txt
    branch :tb:dut:u_fsm  c41e09a7b3d2.0.3  src/fsm.vhd:88   # unreachable: state is one-hot
    instance :tb:u_debug*                                    # debug logic
    file */generated/*.vhd
    unit work.pkg_tools

``yavs cov report DB --exclude exclusions.txt`` leaves those items out of the totals and the
Cobertura report and lists them under "excluded" with their reasons. A row matches by instance
(``*`` and ``?`` allowed), kind, arm and the statement's text hash, so a statement that moves
stays excluded; the file and line are only there to read, and a line that is no longer right
is warning ``W0110``. A row that matches nothing (the statement was edited) is ``W0111``, an
excluded item that did run ``W0114``; ``-Werror`` makes them fail the report. Row types:
``statement``, ``branch``, ``condition INSTANCE HEX12.OCC ROW`` and ``expression INSTANCE
HEX12.OCC ROW`` (one row of a condition or expression; the hash is that of the condition's
text, not of its statement, so editing the body of an ``if`` keeps its row exclusions; ROW is
twice the input's position plus its value, as ``--uncovered`` prints it), ``toggle PATH SCALAR [rise|fall]`` (both directions of one scalar,
or only the 0->1 or only the 1->0 one; ``*`` and ``?`` allowed in path and scalar, so ``toggle
:tb:dut data(*)`` takes the whole vector; ``W0114`` for each excluded direction that ran),
``instance`` (that instance and everything below), ``unit`` (a package) and ``file``; anything
else is error ``E0112``. ``--uncovered`` names the direction when only one never happened. The
database keeps the raw counts.

An exclusion file whose name ends in ``.yaml`` or ``.yml`` is read as YAML instead (any other
name as the rows above; both mean the same). It is a list of entries with a ``kind`` and the
fields of that kind, plus an optional ``reason``:

.. code-block:: yaml

    # exclusions.yaml
    exclude:
      - kind: branch
        instance: ":tb:dut:u_fsm"
        hash: "c41e09a7b3d2.0.3"     # HEX12.OCC, branch: HEX12.OCC.ARM
        line: "src/fsm.vhd:88"       # optional, only shown when it moved
        reason: unreachable, state is one-hot
      - kind: toggle
        instance: ":tb:dut"
        signal: "data(*)"
        direction: fall              # optional: rise or fall, default both
      - kind: instance
        instance: ":tb:u_debug*"
        reason: debug logic
      - kind: file
        file: "*/generated/*.vhd"
      - kind: unit
        unit: work.pkg_tools

``statement`` and ``branch`` take ``instance``, ``hash``, ``line``; ``condition`` and
``expression`` the same plus ``row: N`` with ``hash: "HEX12.OCC"``; ``toggle`` takes
``instance``, ``signal``, ``direction``; ``instance``, ``unit`` and ``file`` take the key of
the same name. Values are always read as text, so ``no``, ``on`` or a hash of digits only need
no care, but quote globs that start with ``*`` or contain ``: ``. An unknown kind or key, a
missing field or invalid YAML is error ``E0112`` with the line. ``yavs cov report --uncovered=yaml
DB`` prints the misses as such a file (the statement as a comment, no reason); redirect it to
a ``.yaml`` file and delete what must stay.

Which tests cover what
~~~~~~~~~~~~~~~~~~~~~~

Each run's database names its test: ``--cov-test=NAME``, by default the enabled test case
when a ``runner_cfg`` generic is given, else the top level with its ``-g`` generics
(``tb(n=0)``). Runs with the same name count as one test. ``yavs cov merge`` keeps, for every
item, the tests that hit it.

``yavs cov rank DB ...`` merges its inputs and prints a small set of tests that reaches the
same coverage, picked greedily (each pick is the test that adds the most items not yet hit),
the tests that set does not need, and per test how many items it hit and how many no other
test hit:

.. code-block:: text

    3 tests hit 26 of 39 items

    minimal set (greedy):
      lib.tb.test_full     +26  26/26

    not needed: lib.tb.test_reset, lib.tb.test_short

    test                    items    unique
    lib.tb.test_full           26         5
    ...

``yavs cov report DB ... --tests`` prints every item that ran as an exclusion row followed by
``# tests:`` and the tests that hit it. Both take ``--exclude``.

HTML reports are not available yet.

Calling C from VHDL
-------------------

A subprogram whose ``foreign`` attribute is ``"VHPIDIRECT symbol"`` calls the C function
``symbol``. ``--load=LIB`` loads a shared library (repeat it for several); symbols are looked
up in the libraries in order, then in the ``yavs`` executable. A symbol that is defined nowhere gives
a warning before the simulation starts (``W0108``, an error with ``-Werror``) and the error
``E0107`` only when the subprogram is called.

.. code-block:: vhdl

    function add1 (x : integer) return integer;
    attribute foreign of add1 : function is "VHPIDIRECT add1";
    function add1 (x : integer) return integer is begin return 0; end; -- placeholder body

.. code-block:: c

    int32_t add1(int32_t x) { return x + 1; }

.. code-block:: bash

    yavs run --work=lib:out/lib --load=./libadd.so tb

Types map to C as follows: an integer subtype to the smallest of ``int8_t`` to ``int64_t``
that holds its range (``integer`` is ``int32_t``), an enumeration to the smallest unsigned type
that holds its literals (``boolean``, ``bit``, ``character`` and ``std_logic`` are
``uint8_t``), ``real`` to ``double``, a physical type to ``int64_t``. A scalar ``out`` or
``inout`` parameter is passed as a pointer. An array is passed as a pointer to its elements
from left to right (``std_logic`` elements as one code byte each, ``'0'`` is 2 and ``'1'`` is
3), and an unconstrained array parameter is followed by one ``int64_t`` length per dimension.
A record of scalars is passed as a pointer to a C struct, an access value as a pointer to the
designated object. ``out`` and ``inout`` values are copied back after the call and checked
against their VHDL type, as is a function result. A word between ``VHPIDIRECT`` and the symbol
(a library name) is ignored; without a symbol the subprogram's name is used. A placeholder
VHDL body is allowed and never runs.

PSL assertions
--------------

yavs runs a simple subset of PSL, written as VHDL-2008 statements in an architecture, block or
generate statement, or, with ``--psl``, in comments that start with ``-- psl``:

.. code-block:: text

   default clock is rising_edge(clk);
   ...
   p1: assert always count < 16 report "count overflow";
   p2: assert never (full and empty);
   p3: assert always req -> next[2] ack severity warning;
   p4: assume always (valid -> ready) @falling_edge(clk);
   c1: cover {state = DONE} report "reached DONE";
   -- psl p5: assert always start -> next busy;

Supported: ``default clock is <condition>;``; ``assert`` and ``assume`` with ``always b``,
``never b``, ``always a -> b``, ``always a -> next b`` and ``always a -> next[n] b`` on VHDL
conditions (``boolean``, ``bit`` or ``std_ulogic``), a property optionally in parentheses;
``cover {b}``; an ``@<condition>`` clock instead of the default clock; ``report`` and ``severity``
clauses. Each directive is checked on every tick of its clock with the signal values at that
tick. A failing ``assert`` or ``assume`` reports like a VHDL assertion, with severity ``error``
and the message ``PSL assertion failed`` unless the directive gives others; a ``cover`` prints
its ``report`` text, as a note, every time it is hit, and nothing without one. A ``-- psl``
comment holds one statement, which may continue on the comment lines below it up to its ``;``.
Without ``--psl`` such comments are comments; pass the option to ``analyze``, ``elab`` and ``run``.

Built-in functions ``prev``, ``stable``, ``rose``, ``fell``, ``isunknown``, ``countones``,
``onehot`` and ``onehot0`` work inside the conditions.

Sequences in braces run in ``cover``, in ``assert never``, and on either side of ``|->`` and
``|=>``:

.. code-block:: text

   c2: cover {req; busy[*2 to 4]; done} report "transfer";
   p6: assert never {valid; not ready[*3]};
   p7: assert always {start; req} |=> {busy[*1 to 3]; ack};
   p8: assert always {write} |-> next[2] ready;

A boolean stands for a sequence of one on the left of ``|->`` and ``|=>``, and with a repetition
without braces (``never a[*3]``); ``[*n]`` first in a sequence means ``true[*n]``.
A sequence has concatenation ``;``, fusion ``:``, alternation ``|``, ``&&`` (both match and end
together), ``&``, ``within``, and ``[*]``, ``[+]``, ``[*n]``, ``[*m to n]``, ``[*m to inf]``,
``[=n]`` and ``[->n]`` repetition; ``|``, ``&``, ``&&`` and ``within`` need braces when mixed.
A ``cover`` hit and a failing ``never`` are reported at the tick where a match ends. The right
side of ``|->`` and ``|=>`` is a boolean or a sequence that must match from the tick the left
side ends (``|->``) or the tick after (``|=>``): it fails at the tick where it can no longer
match. Without ``always`` a directive starts at the first clock tick only. One directive may
test at most 6 different conditions on its right side; a longer one is skipped with ``W0004``.

Properties may also use ``next_a``, ``next_e``, ``next_event``, ``next_event_a``, ``next_event_e``
(``next_event(c)(p)`` counts the current cycle), ``until``, ``until_``, ``before``, ``before_``,
``sync_abort``, ``abort``, ``async_abort``, ``<->`` and ``iff``, ``and`` and ``or`` of properties, nested ``next`` and
``always``, and a property on the right of ``|->`` and ``|=>``:

.. code-block:: text

   p10: assert always (req -> next_e[1 to 4] (ack = '1'));
   p11: assert always {start} |=> (busy until done);

These are weak: an obligation still open when the run ends is not a failure. A failure is
reported at the tick where the obligation can no longer be met, once per tick however many
starts fail; the parts of a conjunction (``next_a``) fail separately. The strong forms
(``eventually!``, ``next!``, ``until!``, ``before!``, a sequence followed by ``!``) are skipped
with ``W0004``. ``abort`` and ``async_abort`` are checked at the clock tick like ``sync_abort``,
not between ticks as the PSL standard defines the asynchronous abort.

Sequences and properties can be named, with ``boolean``, ``const``, ``sequence`` and ``property`` formals,
and used where a sequence or a whole property is expected:

.. code-block:: text

   sequence handshake (boolean r, a) is {r; a};
   property ack_follows (boolean r, a) is always (r -> next a);
   c3: cover handshake(req = '1', ack = '1');
   p9: assert ack_follows(req = '1', ack = '1');

Everything else (other ``property`` and ``sequence`` declarations, other operators such as ``eventually!``, the ``next`` and ``ended`` functions, directives without a clock
and verification units) is skipped with warning ``W0004``, once per design unit; the rest of
the design runs as usual. ``restrict``, ``restrict_guarantee``, ``assume_guarantee``, ``fairness``, ``strong fairness`` and directives that
call ``nondet`` or ``nondet_vector`` are for formal tools only: they are skipped without a
warning, as they have nothing to check in a simulation.

Diagnostics
-----------

Errors and warnings have a stable code, such as ``E1100``. ``yavs --explain E1100`` prints its
explanation. ``-Werror`` turns warnings into errors, ``-Werror=CODE``, ``-Wno-CODE`` and
``-WCODE`` change one code. ``--message-format=json`` prints one JSON object per diagnostic,
``--color=never`` turns colour off (``NO_COLOR`` is honoured).

VHPI plugins
------------

``--vhpi=LIB[:ENTRY]`` (repeatable, loaded in order) loads a VHPI plugin after elaboration and
before the simulation starts. Without ``ENTRY`` every routine of the library's NULL-terminated
``vhpi_startup_routines`` array runs; with it, that function runs. ``vhpi_control`` with
``vhpiStop`` or ``vhpiFinish`` ends the run like ``std.env.stop`` or ``std.env.finish`` with
status 0. A plugin that cannot be loaded is error ``E0901`` or ``E0902`` and exit code 2, before
time 0.

Callbacks for the phases of the simulation run: start and end of simulation, start and end of
initialization, next time step, start of each cycle, start and end of each cycle's processes,
start of postponed processes, last known delta cycle (the read-write
phase: values put here take effect in one more delta), end of time step (the read-only phase,
after postponed processes), each also in its repeating ``Rep`` form, and after a delay. They can
be disabled, enabled again and removed; a disabled callback costs nothing while it waits. VHPI
times count in units of ``--resolution``.

Plugins walk the elaborated design and read values with ``vhpi_get_value`` from a callback:
signals, ports, generics and constants, and their record fields and array elements, in every
standard format (binary, octal, decimal and hexadecimal strings, strings, characters,
enumeration, logic, integer, real, time and physical values and their vectors).
``vhpiObjTypeVal`` reports the object's own format; a buffer that is too small makes the call
return the size it needs. Enumeration values are positions: a ``std_ulogic`` gives ``vhpiU`` to
``vhpiDontCare``, ``bit`` and ``boolean`` give 0 and 1. A format that does not fit the value is a
``W0903`` warning and returns -1, never a converted value. The type of an object
(``vhpiType``, ``vhpiBaseType``, ``vhpiElemType``) shows its kind, name, range constraints,
enumeration literals, record elements and physical units.

``vhpi_put_value`` writes signals and ports and their record fields and array elements, in the
same formats, and an integer into a logic vector (two's complement, the rightmost element the
least significant bit): ``vhpiDepositPropagate`` sets the value until a driver next updates it,
``vhpiForcePropagate`` holds it against the drivers until ``vhpiRelease``, which returns the
signal to its drivers' value. The non-propagating ``vhpiDeposit`` and ``vhpiForce`` are refused
(``W0903``). A write takes effect in the next delta at the same time; in the read-only phases
(start of postponed processes, end of time step, end of simulation) it is refused. Constants, generics, records as a whole and ``vhpiSizeConstraint`` are
``W0903``.

Variables of processes and shared variables (of architectures, blocks and packages, not of a
protected type) and their record fields and array elements are read with ``vhpi_get_value``
and written with ``vhpi_put_value`` in ``vhpiDeposit`` or ``vhpiDepositPropagate`` mode from
simulation callbacks. A deposit changes the variable at once: a read later in the same
callback returns the new value, and the process sees it when it next resumes. A process's
variables exist once it has run, from ``vhpiCbEndOfInitialization`` on; forcing or releasing a
variable is a ``W0903`` warning. A ``vhpiCbValueChange`` callback on a
signal, port or one of their sub-elements runs once in every cycle in which any of its elements
changed, with the object's value in the format its ``value`` field asks for.

A plugin can also implement VHDL subprograms. A subprogram whose ``foreign`` attribute is
``"VHPI library model"`` calls the model that a plugin registered at startup with
``vhpi_register_foreignf`` (``vhpiFuncF`` for a function, ``vhpiProcF`` for a procedure,
``libraryName`` and ``modelName`` equal to the two words). The registration's ``elabf`` runs
once for every subprogram that is bound to it, before the simulation starts, and its ``execf``
at every call, with ``obj`` the call: ``vhpi_iterator(vhpiParamDecls, obj)`` or
``vhpi_handle_by_index(vhpiParamDecls, obj, i)`` give the parameters (``vhpiNameP``,
``vhpiModeP``), ``vhpi_get_value`` reads them in every format, and ``vhpi_put_value`` with
``vhpiDeposit`` or ``vhpiDepositPropagate`` writes the ``out`` and ``inout`` parameters and, on
``obj`` itself, the function result. Parameters and results are scalars or arrays of scalars.
A record parameter's fields (scalars or constrained arrays of scalars) are its
``vhpiSelectedNames``, read and written one by one. An ``in`` access parameter reads as
``vhpiPtrVal``: the address of the designated object's data (an ``integer`` is 64 bits), NULL
for ``null``. A signal parameter reads the signal's value; for an ``out`` or ``inout`` one,
``vhpi_put_value`` deposits a value on the actual and ``vhpi_schedule_transaction`` on the
parameter's driver (``vhpi_iterator(vhpiDrivers, param)``) schedules a transaction, with a
delay in resolution units and ``vhpiInertial`` or ``vhpiTransport``; both take effect when the
call returns. File parameters are not supported. A function whose result
is an unconstrained array first sets its length with ``vhpi_put_value`` in
``vhpiSizeConstraint`` mode (``numElems``), then puts the value; the result is indexed ``1 to
n``. A constrained result subtype has its own length and bounds. A function that sets no
result, or a value that does not fit its VHDL type, is a run-time error. A model nobody
registered is warning ``W0108`` before the simulation and error ``E0107`` only when the
subprogram is called. ``vhpi_get_foreignf_info`` returns a registration. Calls are serialized,
so a plugin needs no locking, whatever ``--threads`` says.

A plugin can implement an architecture too. An architecture whose ``foreign`` attribute is
``"VHPI library model"`` (or ``"VHPI library;model"``) has only its ports elaborated, plus its
entity's declarations and statements; the model a plugin registered with ``vhpiArchF`` stands
in for the rest. Before the simulation starts, the registration's ``elabf`` runs for every
instance, then its ``execf``, each with ``obj`` the instance (``vhpi_get_foreignf_info`` on it
returns the registration). While they run, the model creates a process with
``vhpi_create(vhpiProcessStmtK, instance, NULL)`` (the instance or its design unit) and drivers
for its ``out`` and ``inout`` ports with ``vhpi_create(vhpiDriverK, port, process)``. It drives
them with ``vhpi_schedule_transaction(driver, value, 1, delay, vhpiInertial or vhpiTransport,
pulse_rejection)``, from ``execf`` or from callbacks such as ``vhpiCbValueChange`` on its
``in`` ports. An instance whose model no plugin registered is error ``E0905``.

Two more kinds of model are not tied to a declaration. A ``vhpiLibF`` (library) model's ``elabf``
runs once when the first foreign subprogram or architecture whose library word equals its
``libraryName`` is bound, and its ``execf`` once at the start of the simulation. A ``vhpiAppF``
(application) model's ``elabf`` and ``execf`` run once at the start of the simulation, whatever
the design contains. All of them run before the ``vhpiCbStartOfSimulation`` callbacks, with
``obj`` the registration (``vhpi_get_foreignf_info`` on it returns the registration). A library
no foreign model of the design names never runs.

Plusargs
--------

Arguments of the form ``+NAME`` or ``+NAME=VALUE`` after the top level are accepted by ``yavs
run`` and kept as the tool's arguments, in order: ``yavs run --work=lib:out/lib --vhpi=./plugin.so
tb +seed=7 +verbose``. A plugin reads them with ``vhpi_get(vhpiArgcP, tool)`` and the
``vhpiArgvs`` iterator (``vhpi_get_str(vhpiStrValP, arg)``); the first argument is the program
name ``yavs``. VHDL code cannot see them. Any other argument after the top level, apart from
``-gNAME=VALUE``, is a usage error (exit 2).

Debugging
---------

``yavs run --dap=PORT`` runs the design under a debugger that speaks the Debug Adapter Protocol
(DAP), the protocol of most editors' debug views. yavs listens on ``127.0.0.1:PORT`` for one
client and prints ``yavs: DAP listening on 127.0.0.1:PORT`` on stderr first; ``--dap=0`` picks a
free port. ``--dap=stdio`` talks DAP on standard input and output instead, for clients that start
the debugger themselves. The design given on the command line is the one debugged; the client's
``launch`` request only starts it (``"stopOnEntry": true`` stops at the first statement).

.. code-block:: bash

    yavs run --work=lib:out/lib --dap=4711 tb

In the editor, attach to the port (for example with a ``debugServer`` launch setting).

Graphical debugger
~~~~~~~~~~~~~~~~~~

``yavs run --gui`` opens the run in ``yavs-gui``, the graphical debugger shipped next to ``yavs``
(found there, else on ``PATH``; missing is error ``E0122``). The other run arguments are passed on
unchanged:

.. code-block:: bash

    yavs run --gui --work=lib:out/lib tb --stop-time=1ms

``yavs-gui -- <run arguments>`` does the same, and ``yavs-gui --attach 127.0.0.1:4711`` connects to
a ``yavs run --dap=4711`` started by hand. The window has Run, Pause and Stop, a console with the
run's reports, and docked panels for the hierarchy, source, waves, call stack and variables, which
are being filled in step by step; closing the window ends the simulation.

VS Code
~~~~~~~

The extension in ``editors/vscode`` adds a ``yavs`` debugger: it analyzes the files you list,
starts ``yavs run --dap=stdio`` and lets you set breakpoints in ``.vhd`` and ``.vhdl`` files.
yavs must be on ``PATH`` (or set ``yavsPath``). Build and install it once:

.. code-block:: bash

    cd editors/vscode
    npx @vscode/vsce package --out yavs-debug.vsix
    code --install-extension yavs-debug.vsix

(or open ``editors/vscode`` in VS Code and press F5 to try it from source). Then add a launch
configuration to the testbench's workspace, ``.vscode/launch.json``:

.. code-block:: json

    {
      "version": "0.2.0",
      "configurations": [
        {
          "type": "yavs",
          "request": "launch",
          "name": "Debug tb",
          "top": "tb",
          "work": "work:${workspaceFolder}/.yavs/work",
          "sources": ["${workspaceFolder}/pkg.vhd", "${workspaceFolder}/tb.vhd"],
          "libraries": [],
          "std": "2008",
          "generics": { "WIDTH": "8" },
          "args": ["--stop-time=1ms"],
          "stopOnEntry": false
        }
      ]
    }

``sources`` are analyzed into ``work`` in order before every session (leave it empty to analyze
yourself); ``libraries`` are more ``NAME:PATH`` mappings, ``generics`` become ``-gNAME=VALUE`` and
``args`` go to ``yavs run`` as they are. Set a breakpoint in the testbench, pick "Debug tb" in the
Run and Debug view and press F5: the call stack lists one thread per process, the Variables view
shows locals, signals and generics, hovering a signal shows its value, and reports go to the
Debug Console. With ``"port": 4711`` instead, the extension attaches to a ``yavs run
--dap=4711`` started by hand (see the recipe below).

Recipe: debug one VUnit test from VS Code
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

``yavs run --dap=PORT`` listens first and holds the simulation until a client attaches, so a test
runner can start it and VS Code can join afterwards. It prints ``yavs: DAP listening on
127.0.0.1:PORT`` and then ``yavs: waiting for a debugger on port PORT`` on stderr. The runner's
own output and exit status work as in a normal run: when you disconnect or the run ends, the test
is reported as usual.

1. Add an attach configuration to ``.vscode/launch.json`` (the snippet "yavs: Attach to yavs"
   inserts it):

   .. code-block:: json

       { "type": "yavs", "request": "launch", "name": "Attach to yavs", "port": 4711 }

2. Open the testbench and set breakpoints.
3. In a terminal, start the one test with the debug flag of the VUnit simulator interface for
   yavs. It runs the test with ``--dap=4711`` (or the port in ``YAVS_DAP_PORT``) and prints the
   waiting line:

   .. code-block:: bash

       python run.py lib.tb_fifo.test_full --debug

   Exactly one test must be selected and ``-p`` must be 1, since only one run can own the port;
   ``--debug`` cannot be combined with ``--gui``.
4. Pick "Attach to yavs" in the Run and Debug view and press F5. The breakpoints are sent
   before the run starts, the run continues and stops on the first one. Continue, step, inspect
   as in any session.
5. Disconnect, or let the test finish: VUnit then prints the result of the test.

The run also writes its waveform to ``wave.fst`` in the test's output directory (the path is in
the ``yavs run`` line VUnit prints). Under ``--dap`` yavs flushes the file at every stop, so at a
breakpoint or step it holds every change of the completed time steps up to the stop (the time
step in progress shows once it ends). To follow it next to the debug session, open the file with
a VS Code waveform extension that opens FST files and add the signals you want. Viewers with an
auto-reload option or a reload button need it after each stop; a viewer without one needs the file
closed and reopened. The same works for any run: ``yavs run --dap=4711 --wave=out.fst tb``.

* **Threads are processes**, named by their path such as ``:tb:u1:p``.
* **Line breakpoints** stop at the statements that start on the line, or on the next line that
  has one; a hit condition (``3``, ``>= 3``, ``% 3``) is supported.
* **Conditions** on line and signal breakpoints are VHDL boolean expressions over the locals of
  the stopped frame and the signals visible from the process (or names by path): ``count = 3
  and state = IDLE``, ``data(7) = '1'``, ``now_t >= 20 ns``, ``r.valid``. Relations, ``and``,
  ``or``, ``not`` and the others on ``boolean`` and ``bit``, arithmetic on integers, reals and
  physical values, fields and elements are supported; a ``bit`` or ``std_ulogic`` result counts
  as ``'1'``. Function calls and attributes are not: such a breakpoint is shown as unverified
  with the reason. An error while evaluating stops with the message.
* **Stepping** within the stopped process: step over (``next``), into a subprogram (``stepIn``)
  and out of it (``stepOut``). Stepping over a ``wait`` stops when the process resumes. After a
  pause between simulation cycles, a step stops at the next statement of any process.
* **Signal breakpoints** stop after the simulation cycle in which a signal changed: in VS Code
  "Break on Value Change" on a signal in the Variables view (DAP data breakpoints, also on a
  name or path through ``dataBreakpointInfo``). "yavs: Break when a signal equals..." stops only
  when the signal takes a value, a VHDL literal of its type (see values below); other clients
  send the custom request ``yavs/setSignalBreakpoints``. The stop gives the signal's path and new
  value.
* **Force, deposit and release** a signal or a part of one (a field, an element or a slice:
  ``r.a``, ``v(3)``, ``d(7 downto 4)``; only that part changes): "Force a signal...", "Deposit a signal value..." and
  "Release a forced signal..." in the context menu of a signal in the Variables view or as
  "yavs:" commands (custom requests ``yavs/force``, ``yavs/deposit`` and ``yavs/release`` for
  other clients). A force holds the effective value (mode ``in``, what readers see) or the
  driving value (mode ``out``) until it is released; a deposit sets the value until a driver
  next changes it. The change is made in the next simulation cycle, also from a stop inside a
  process. "Set Value" in the Variables view deposits a signal and writes a variable at once.
* **Values** are VHDL literals of the target's type: ``'1'``, ``42``, ``16#FF#``, ``TRUE``,
  ``1.5``, ``10 ns`` (any unit of a physical type), ``"01ZX"``, bit strings such as ``x"A5"`` or
  ``8ux"F"``, and aggregates with positional and named elements, ranges and ``others``:
  ``(a => 1, b => x"F")``, ``(7 downto 4 => '1', others => '0')``.
* **Pause** stops a running simulation: inside a process if one is running, otherwise between
  cycles.
* **Run for a time or until a time**, **step to the next delta** and **step to the next time
  step** (VS Code commands "yavs: Run for...", "yavs: Run until...", "yavs: Step to the next
  delta" and "yavs: Step to the next time step"; custom requests ``yavs/runFor``,
  ``yavs/runUntil``, ``yavs/stepDelta``, ``yavs/stepTimestep`` for other clients). Times are
  TIME literals such as ``10 ns`` or ``1us``. A breakpoint on the way still stops, and continuing
  from it resumes the run up to its time. A delta step from a stop inside a process finishes the
  current simulation cycle; a time step runs the next cycle and every delta after it at that
  time.
* **Call stack**: one frame per subprogram with its source position; a waiting process shows its
  ``wait`` statement.
* **Variables**: the locals of each frame (process variables, parameters, subprogram variables),
  the signals and ports of the regions around the process, and the generics of its instance.
  Records expand by field and arrays by index (large arrays in pages); "Set Value" works on a
  field or an element too.
* **Hierarchy**: a scope listing the design's instances, blocks and generates, each with its
  processes (and their state) and signals; the custom request ``yavs/hierarchy`` gives the same
  tree to other clients.
* **Evaluate** reads a local, a signal visible from the process, or a signal by path
  (``:tb:u1:s`` or ``tb.u1.s``).
* Every stop says its simulation time and delta, and reports and ``OUTPUT`` text appear in the
  debug console.
* **Live waves**: see below.

A stop is a defined point of the simulation: under ``--dap`` processes run on one thread, so
``--threads`` is ignored (warning ``W0109``) and every session behaves the same. When the
simulation ends the client gets the exit status of the run; disconnecting ends the simulation
(exit status 1 if it had not ended).

``--wave`` and ``--vhpi`` work under ``--dap``: the waveform file is the one the same run
without the debugger writes, finished when the session ends. A VHPI plugin's own prints go to
standard output, which ``--dap=stdio`` uses for the protocol, so debug a run with ``--vhpi``
over a port.

**Live waves** for a waveform viewer: the custom request ``yavs/waveSubscribe`` selects signals
(and variables) with the globs and limits of ``--wave-include``, ``--wave-exclude``,
``--wave-max-array`` and ``--wave-variables`` and answers with the recorded hierarchy, as the
waveform file declares it. From then on ``yavs/waves`` events carry the final value of every
changed signal per time step, as base64 of the value bytes (the hierarchy gives each signal's
``encoding`` and type, so a client can decode them), sent at least every
``flushMs`` (default 100) milliseconds and always before a stop or the end, so the waves reach
the stop time. A slow client never holds up the simulation: if it falls behind, batches are
merged and only the last change of each value in them is kept. ``yavs/waveUnsubscribe`` ends
the stream.

Not yet: function calls and attributes in breakpoint conditions, indices of enumeration-indexed
arrays in a forced part, and ``--dap`` together with ``--cov``.
