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
* ``--wave out.fst`` records the whole design as an FST file that waveform viewers read. It is
  finished on normal end, on ``--stop-time`` and on Ctrl-C. ``--wave-include GLOB`` and
  ``--wave-exclude GLOB`` (repeatable, matching paths such as ``tb.u1.count``) choose what is
  recorded, and arrays larger than ``--wave-max-array`` (default 65536) are left out with a
  warning. Several delta cycles at one time step give one change with the final value.

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
``vhpi_startup_routines`` array runs; with it, that function runs. The plugin's start and end of
simulation callbacks fire, and ``vhpi_control`` with ``vhpiStop`` or ``vhpiFinish`` ends the run
like ``std.env.stop`` or ``std.env.finish`` with status 0. A plugin that cannot be loaded is error
``E0901`` or ``E0902`` and exit code 2, before time 0. The object model behind the plugin API is
still a stub: it offers the design root, the tool and these callbacks.

Not implemented yet
-------------------

``--dap`` is accepted but reports that it is not implemented yet, instead of being ignored.
