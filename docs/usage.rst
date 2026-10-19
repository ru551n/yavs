Command line
============

yavs works like the other command line simulators: analyze the files into libraries, then
elaborate and run a top level. ``yavs --help`` and ``yavs <command> --help`` list every option.

Libraries
---------

A library is a directory, given as ``NAME:PATH``. ``--work`` is the library a command works in,
``--map`` adds another one (repeatable). A missing or empty directory is an empty library;
``analyze`` creates it. ``std`` and ``ieee`` are always available and cannot be mapped.

Analysis is cheap to repeat: a library records which file holds which unit, and ``elab`` and
``run`` analyze the units the top level needs from source, so there is no compiled library to
keep up to date.

Analyze
-------

.. code-block:: bash

    yavs analyze --std=2019 --work=lib:out/lib pkg.vhd tb.vhd

The files are analyzed in order; the first failing file stops the batch. ``--std`` is the
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
  cleared the assert counts since.
* ``--threads N`` evaluates processes on N threads (default 1). The output is identical for any
  N. ``--shuffle[=SEED]`` randomizes the process order to find testbenches that depend on it.
* ``--tier=llvm`` compiles every function with LLVM before time 0 and runs that code; the output
  is the same as with the default ``--tier=interp``. It needs a yavs built with the ``llvm``
  feature (LLVM 21). ``--verbose`` prints what the LLVM tier leaves to the interpreter.
* ``--resolution=ps`` sets the time resolution limit (``fs``, ``ps``, ``ns``, ``us``, ``ms`` or
  ``sec``; default ``fs``). It is what ``std.env.resolution_limit`` returns and the unit of TIME
  values in ``std.reflection`` mirrors. A TIME literal with a unit below the limit anywhere in
  the elaborated design (``1 ps`` under ``--resolution=ns``), in a package it uses or in code that
  never runs, is an error (``E2120``, LRM 5.2.4.2) that lists the locations; ``0 ps`` is allowed,
  and so is ``0.4 ns``, because the rule is about the unit and not the value.
* ``--wave out.fst`` records waveforms, see `Waveforms`_.
* ``--cov[=FILE]`` counts code coverage, see `Code coverage`_.

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
* FST has no delta cycles: several deltas at one time step give one change with the final
  value. The file is the same for any ``--threads``.
* ``--wave-include GLOB`` and ``--wave-exclude GLOB`` (repeatable) choose what is recorded by
  path, such as ``tb.u1.count``: matching ignores case, ``*`` matches within one level, ``**``
  across levels and ``?`` one character, and a pattern that matches a scope covers everything
  below it. Signals left out cost nothing while simulating.
* ``--wave-variables`` also records the scalar variables of processes (integers, enumerations,
  ``real``, ``time``, ``std_logic``), in a scope named after the process, such as
  ``tb.main.count``, and the include and exclude globs apply to them. A variable is sampled at
  the end of each time step, so the file shows its value after all the deltas of that step and
  is the same for any ``--threads``. Without the option, variables are not recorded and cost
  nothing. Arrays, records and shared variables are not recorded yet.
* Arrays and vectors with more elements than ``--wave-max-array`` (default 65536) are left out
  with warning ``W0802``.
* The file is written out periodically while simulating and finished at the end of the
  simulation, at ``--stop-time`` and on Ctrl-C: the first Ctrl-C ends the run after the current
  cycle and exits with status 1, a second one exits at once (the file is then readable up to
  its last periodic write). A path that cannot be written is error ``E0801``.

Code coverage
-------------

.. code-block:: bash

    yavs run --work=lib:out/lib --cov=test_a.ycov tb -gtest=a
    yavs run --work=lib:out/lib --cov=test_b.ycov tb -gtest=b
    yavs cov merge test_a.ycov test_b.ycov -o all.ycov
    yavs cov report all.ycov --cobertura coverage.xml

``--cov[=FILE]`` counts how often every statement and every branch of the design ran, and writes
the counts to the coverage database ``FILE`` (default ``yavs.ycov``) at the end of the run,
whatever its outcome. Without ``--cov`` there are no counters and no cost.

* **Statements**: every sequential statement and every concurrent statement (each time it
  runs). Declarations are not counted.
* **Branches**: each arm of an ``if``/``elsif``/``else`` (an ``if`` without ``else`` has an arm
  for "no condition held"), each ``case`` alternative, each choice of a conditional
  (``when ... else``) or selected (``with ... select``) assignment, sequential or concurrent
  (one without a final ``else`` has the "no condition held" arm too), and the body of a ``for``
  or ``while`` loop (counted per iteration).
* Code of an architecture is counted per instance and shown under its path (``:tb:u_dut``),
  blocks and generate iterations included. Code of a package is counted once under
  ``library.package``. The ``std`` and ``ieee`` libraries are not counted.
* Items are identified by their path and the text of their statement, not by its position in
  the file, so a database stays mergeable after edits elsewhere in the file. Counts are the same
  for any ``--threads``.

``yavs cov merge A.ycov B.ycov ... -o OUT.ycov`` adds up the counts of several runs: write one
database per run (one ``--cov=FILE`` each) and merge them. ``yavs cov report DB ...`` merges its
inputs, prints statement and branch totals, one line per instance or package, and every
statement or branch that never ran with its file and line; ``--cobertura FILE.xml`` also writes a
Cobertura XML report (one entry per source line: the hits of its most executed statement, and
how many of its branches ran) for CI services that show coverage. A database that cannot be read
is error ``E0111``, a file that cannot be written ``E0110``.

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
``statement``, ``branch``, ``instance`` (that instance and everything below), ``unit`` (a
package) and ``file``; anything else is error ``E0112``. The database keeps the raw counts.

Toggle coverage and HTML reports are not available yet.

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

Everything else (``property`` and ``sequence`` declarations, sequences such as
``{a; b}``, other operators, built-in functions such as ``rose``, directives without a clock,
``restrict``, ``fairness`` and verification units) is skipped with warning ``W0004``, once per
design unit; the rest of the design runs as usual.

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
``obj`` itself, the function result. Parameters and results are scalars or arrays of scalars;
records, access values, signal and file parameters are not supported. A function that sets no
result, or a value that does not fit its VHDL type, is a run-time error. A model nobody
registered is warning ``W0108`` before the simulation and error ``E0107`` only when the
subprogram is called. ``vhpi_get_foreignf_info`` returns a registration. Foreign architectures
(``vhpiArchF``) can be registered and queried but are not run yet. Calls are serialized, so a
plugin needs no locking, whatever ``--threads`` says.

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
--dap=4711`` started by hand.

* **Threads are processes**, named by their path such as ``:tb:u1:p``.
* **Line breakpoints** stop at the statements that start on the line, or on the next line that
  has one; a hit condition (``3``, ``>= 3``, ``% 3``) is supported.
* **Stepping** within the stopped process: step over (``next``), into a subprogram (``stepIn``)
  and out of it (``stepOut``). Stepping over a ``wait`` stops when the process resumes. After a
  pause between simulation cycles, a step stops at the next statement of any process.
* **Signal breakpoints** stop after the simulation cycle in which a signal changed: in VS Code
  "Break on Value Change" on a signal in the Variables view (DAP data breakpoints, also on a
  name or path through ``dataBreakpointInfo``). "yavs: Break when a signal equals..." stops only
  when the signal takes a value, written as the Variables view shows it (``'1'``, ``"0101"``,
  ``42``, ``TRUE``); other clients send the custom request ``yavs/setSignalBreakpoints``. The
  stop gives the signal's path and new value.
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
* **Evaluate** reads a local, a signal visible from the process, or a signal by path
  (``:tb:u1:s`` or ``tb.u1.s``).
* Every stop says its simulation time and delta, and reports and ``OUTPUT`` text appear in the
  debug console.

A stop is a defined point of the simulation: under ``--dap`` processes run on one thread, so
``--threads`` is ignored (warning ``W0109``) and every session behaves the same. When the
simulation ends the client gets the exit status of the run; disconnecting ends the simulation
(exit status 1 if it had not ended).

Not yet: conditions on breakpoints, forcing and depositing signal values,
live waveforms, composite variables (shown as ``(...)``), and ``--dap``
together with ``--wave``, ``--vhpi`` or ``--cov``.
