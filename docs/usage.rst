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
* ``--resolution=ps`` sets the time resolution limit (``fs``, ``ps``, ``ns``, ``us``, ``ms`` or
  ``sec``; default ``fs``). It is what ``std.env.resolution_limit`` returns and the unit of TIME
  values in ``std.reflection`` mirrors. A TIME literal with a unit below the limit anywhere in
  the elaborated design (``1 ps`` under ``--resolution=ns``), in a package it uses or in code that
  never runs, is an error (``E2120``, LRM 5.2.4.2) that lists the locations; ``0 ps`` is allowed,
  and so is ``0.4 ns``, because the rule is about the unit and not the value.
* ``--wave out.fst`` records waveforms, see `Waveforms`_.

Waveforms
---------

.. code-block:: bash

    yavs run --work=lib:out/lib --wave=tb.fst --wave-exclude='tb.u_ddr.**' tb

``--wave FILE.fst`` records the whole design as an FST file that waveform viewers read:

* The hierarchy follows the elaborated design: instances (shown with their ``entity(arch)``),
  blocks, generate scopes (``g(3)`` for each for-generate index) and package
  signals (``work.pkg.sig``). Names keep the spelling of the source.
* Every signal and port is recorded, ports with their direction. ``std_logic`` and ``bit``
  values and their vectors show as characters (``U``, ``X``, ``Z``...) with the vector bounds
  and direction kept; other enumerations show their literal names; integers, ``time`` and
  ``real`` show as numbers. Records are scopes with one entry per field, and arrays of anything
  other than ``std_logic`` or ``bit`` are scopes with one entry per element (``mem(2)``,
  ``grid(1,2)``). Variables are not recorded.
* FST has no delta cycles: several deltas at one time step give one change with the final
  value. The file is the same for any ``--threads``.
* ``--wave-include GLOB`` and ``--wave-exclude GLOB`` (repeatable) choose what is recorded by
  path, such as ``tb.u1.count``: matching ignores case, ``*`` matches within one level, ``**``
  across levels and ``?`` one character, and a pattern that matches a scope covers everything
  below it. Signals left out cost nothing while simulating.
* Arrays and vectors with more elements than ``--wave-max-array`` (default 65536) are left out
  with warning ``W0802``.
* The file is written out periodically while simulating and finished at the end of the
  simulation, at ``--stop-time`` and on Ctrl-C: the first Ctrl-C ends the run after the current
  cycle and exits with status 1, a second one exits at once (the file is then readable up to
  its last periodic write). A path that cannot be written is error ``E0801``.

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
generate statement, or in comments that start with ``-- psl``:

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
initialization, next time step, start of each cycle, last known delta cycle (the read-write
phase: values put here take effect in one more delta), end of time step (the read-only phase,
after postponed processes), each also in its repeating ``Rep`` form, and after a delay. They can
be disabled, enabled again and removed; a disabled callback costs nothing while it waits. VHPI
times count in units of ``--resolution``. The object model behind the plugin API is still a
stub: it offers the design root and the tool, not yet signals or value-change callbacks.

Not implemented yet
-------------------

``--dap`` is accepted but reports that it is not implemented yet, instead of being ignored.
