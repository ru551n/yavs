Verilog
=======

yavs reads Verilog (IEEE 1364-2005) next to VHDL. The aim is to instantiate readable vendor Verilog,
such as primitive libraries and IP models, from a VHDL testbench.

.. note::

   This is under construction. ``yavs analyze`` reads Verilog files and records their modules in a
   library, and a VHDL testbench can instantiate a Verilog module and simulate it, with the subset of
   Verilog listed under `Simulating Verilog from VHDL`_. A construct outside that subset stops
   elaboration with ``E4501``, which names it. Only the interpreter runs Verilog for now; VHDL keeps
   its compiled tiers. SystemVerilog is not read.

.. contents:: On this page
   :local:

Analyzing Verilog
-----------------

``yavs analyze`` takes ``.v`` and ``.vh`` files (and ``.vp``, which holds protected IP; see below)
together with VHDL files, in one call and one library. The language follows the extension;
``--lang=verilog`` reads every file of the call as Verilog and ``--lang=vhdl`` as VHDL.

.. code-block:: bash

    yavs analyze --work=cells:build/cells -I include -D SIM defs.vh lut.v ff.v

The call above prints ``W4006`` for each module of files that have no ``timescale`` directive, and
records ``LUT2`` and ``ff`` in the library ``cells``.

* Macros, ``timescale`` and ``default_nettype`` persist from one file of the call to the next, as
  the standard describes for files compiled together. Give the files in the order they depend on
  each other.
* ``-I DIR`` (repeatable) is searched for ``include`` files after the directory of the including file.
* ``-D NAME`` or ``-D NAME=VALUE`` defines a macro before the first file. ``-D NAME=VALUE`` also
  defines a VHDL conditional analysis identifier for the VHDL files of the call; ``-D NAME`` alone
  is for Verilog only.
* A file that gives no ``timescale`` before its first module gets ``1ns/1ps``, with warning
  ``W4006``. ``--timescale UNIT/PRECISION`` (for example ``--timescale 1ps/1ps``) sets another one.
* A Verilog option with no Verilog file in the call is ``E0141``.

Names
-----

Verilog is case sensitive, VHDL is not. A library keeps a module under the name as written. A
lookup by name finds the module spelled exactly so first, then the one module that differs only in
case; two such modules are an error that lists them. A Verilog module and a VHDL design unit may not
share a name in one library, whatever the case (``L0035``).

Protected IP
------------

yavs never decrypts protected (IEEE 1735) Verilog. A file with a protected module body is analyzed
with warning ``W4007`` (one per file), and the module is recorded under its name, so a whole directory
of vendor files analyzes. Using the module in a design is error ``E4001``, which names the instance.
Plain text between ``pragma protect begin`` and ``end`` (marked for encryption but not encrypted) is
compiled, with warning ``W4009`` and the markers ignored.

SystemVerilog
-------------

SystemVerilog is not read. A ``.sv`` or ``.svh`` file is ``E4002``, and a SystemVerilog keyword in
a Verilog file is ``E4003``.

Elaboration tasks
-----------------

``$error``, ``$warning``, ``$info`` and ``$fatal`` are read where a module item or a generate-block
item may stand, for example in a ``generate`` ``if`` that rejects a bad parameter. This goes beyond
IEEE 1364-2005, which has no elaboration tasks; vendor and cell models use them. They are recorded,
and a module that has one stops elaboration with ``E4501``: running them is not implemented yet. Any
other system task outside a procedural block is still a syntax error (``E4100``).

What is checked
---------------

The analyzer checks the rules of IEEE 1364-2005 that need no elaboration, and the repository keeps one test per rule, with
its clause. Numbers must be well formed (``E4014``): a based constant has a non-zero size, at least one digit, no leading
underscore, and only digits of its base. Compiler directives must be complete and balanced (``E4004`` to ``E4011``): ``line``,
``unconnected_drive``, ``pragma``, ``elsif`` and ``begin_keywords`` have their operands, ``begin_keywords`` also selects the reserved
words of the version it names, and a directive cannot be redefined as a macro. Declarations are checked per scope (``E4203``): a
module, task, function, named block, generate block or specify block declares a name once, and the ports of a module header are
not declared again in its body. A drive or charge strength goes where the standard puts it, gates take the terminals, strengths
and delays their kind takes, user-defined primitives have one output, first, at least one input, only the table symbols each field
allows and at most one edge per row, a path delay has 1, 2, 3, 6 or 12 values, specify blocks hold timing checks with the right number
of arguments, an input or inout port is not a ``reg`` (``E4202``), a ``case`` has one ``default`` and an item, a function has input
arguments only and no timing control, nonblocking or procedural continuous assignment, event trigger or task enable, a task enable
has an expression for every argument, a generate block has no ports, parameters or specify block,
and the variable of a generate ``for`` loop is a ``genvar`` (``E4204``); these are syntax errors (``E4100``) unless a code is given.
Three rules are warnings, because other tools accept the code: ``t();`` for a task without arguments (``W4205``), declarations in a block without a name (``W4206``) and a variable given a value where it is declared inside a task or block (``W4207``). Rules that need the type of an operand or the elaborated design are left out, for example the name of an instance of a module (only
gates and primitives may omit it), the operands of a real expression and more than one driver of a ``uwire``; ``docs/design/lrm-checks.md``
lists each of them.

Simulating Verilog from VHDL
----------------------------

The top level is VHDL. A VHDL component instance that no configuration and no entity of the
component's name binds is bound to the Verilog module of that name, searched in the work library and
then in each library named with ``-L LIB`` on ``elab`` or ``run``, in that order. The component gives
the module's generics and ports, matched by name (the name as written first, then the one that differs
only in case). A configuration that says ``use open`` leaves the instance unbound, as in VHDL.

``docs/examples/verilog`` holds a counter written in Verilog and a VHDL testbench for it:

.. code-block:: bash

    yavs analyze --work=work:build/work counter.v counter_tb.vhd
    yavs run --work=work:build/work counter_tb

.. code-block:: text

    ** Note: 15000000fs+0: q = 1
        Process counter_tb.process (counter_tb.vhd:19)
    ** Note: 25000000fs+0: q = 2
        Process counter_tb.process (counter_tb.vhd:19)
    ** Note: 35000000fs+0: q = 3
        Process counter_tb.process (counter_tb.vhd:19)

A module can also be instantiated directly, as ``entity lib.module``, with no component declaration:

.. code-block:: vhdl

    dut : entity work.counter generic map (WIDTH => 8) port map (clk => clk, rst => rst, q => q);

The generics and ports come from the module. The library is the one named, as in VHDL (``work`` or a mapped
library); when it does not have the module, the libraries of ``-L LIB`` (on ``analyze``, and on ``elab`` and ``run``
for the instance to be found again) are searched in order, and a module of the named library always wins. The name
is the module's as written first, then the one module that differs only in case, and the message of a miss lists the
libraries searched. Each parameter that is not local is a generic: ``integer`` when its default is a plain integer,
``real``, ``string`` for a string, and ``std_logic_vector`` of its width otherwise (also for one bit). A generic left
out of the generic map keeps the module's default, and so does what depends on it. A scalar port is ``std_logic``
and a vector ``std_logic_vector`` with the module's bounds, so a descending Verilog range is ``downto`` and an
ascending one ``to``. A bound that is a constant expression of integer parameters follows the generic map: ``+``,
``-``, ``*``, ``/``, ``%``, ``**``, ``<<``, ``>>``, comparisons, ``&&``, ``||``, ``!``, ``?:``, ``$clog2``, local
parameters of these and parameters that depend on others (``parameter B = W * 2``). Any other bound that names a
parameter (a bitwise operator, a function call) is recorded as such: with no generic map the defaults give it, and
with one the instantiation is ``E4706``, naming the port. Names that differ only in case (``D`` and ``d``) are
extended identifiers in the map: ``\\D\\ => 64``. Generic and port maps are checked as for a component, with every
form of actual; an error in them is ``E4704`` (generics) or ``E4705`` (ports), naming the module and the parameter or
port, with the Verilog declaration and the VHDL one under it. A module with a ``real`` port, a port expression or an
encrypted body has no such view (``E4703``; use a component of your own), and naming an architecture is ``E4702``.
A user-defined primitive cannot be instantiated from VHDL.

A module in another library is found with ``-L``, the library mapped as usual:

.. code-block:: bash

    yavs analyze --work=cells:build/cells counter.v
    yavs analyze --work=work:build/work counter_tb.vhd
    yavs run --work=work:build/work --map=cells:build/cells -L cells counter_tb

Elaboration reads each Verilog file again with what its analysis used: the ``-D``, ``-I`` and
``--timescale`` options and the macros that earlier files of the same ``analyze`` call defined. A file
or an included file that changed since it was analyzed is ``E4508``: analyze it again.

**Generics and ports.** A generic sets the parameter of its name (``E4701`` when there is none, or it is
a ``localparam``). ``integer`` gives a 32-bit signed value, ``real`` a real, ``string`` 8 bits per
character, ``std_logic_vector`` and ``bit_vector`` a vector of their length, ``std_logic``, ``bit`` and
``boolean`` one bit. A parameter without a type or range takes the type of that value, one with a range
keeps its range. Ports are ``std_logic``, ``std_ulogic``, ``bit`` and vectors of them, of mode ``in``
or ``out``, with the width of the Verilog port (``E4700`` otherwise); the left element of a VHDL vector
is the left (most significant) bit of the Verilog vector. A Verilog input with no VHDL port is ``z``.
VHDL values reach Verilog as ``0``, ``1``, ``x`` and ``z`` (``L`` and ``H`` as ``0`` and ``1``, ``U``,
``W`` and ``-`` as ``x``); a ``bit`` port takes ``x`` and ``z`` from Verilog as ``'0'``.

**Timing.** Every Verilog process of the design runs in one scheduler with the active, inactive and
non-blocking regions of the standard, inside one delta cycle of the VHDL simulation: blocking
assignments and continuous assignments take no delta. What the active region changes reaches VHDL one
delta later, what non-blocking assignments change one delta after that. A Verilog clock that clocks
both a Verilog register and a VHDL flip-flop therefore gives the VHDL flip-flop the register's value from
before the edge. Outputs show the module's initial values (``x`` for a ``reg``, ``z`` for an undriven
net) one delta into the run. Delays are in each module's ``timescale`` unit, rounded to its precision.

**What runs.** Parameters and ``localparam`` (also wide and string values); ``wire``, ``tri``, ``tri0``, ``tri1``,
``supply0``, ``supply1``, ``uwire``, ``reg``, ``integer`` and ``time`` declarations; continuous assignments and net declaration assignments;
``always`` and ``initial`` blocks with ``@(...)``, ``@*``, ``#`` and ``wait``; blocking and non-blocking
assignments, also with an intra-assignment delay; ``if``, ``case``, ``casez``, ``casex``, ``for``,
``while``, ``repeat`` and ``forever``; every operator of the standard on 4-state values of any width,
with the standard's sizing and sign rules; ``$display``, ``$write``, ``$finish``, ``$time``,
``$stime``, ``$realtime``, ``$clog2``, ``$signed`` and ``$unsigned``. The ports of a Verilog instance are
in the wave file under the instance.

**Nets with several drivers.** ``wire`` and ``tri`` nets with more than one driver resolve them as clause
4.6.1 describes: equal values stay, ``0`` and ``1`` give ``x``, ``z`` yields to the other driver. This includes
assignments that drive ``z`` (``assign y = en ? a : 1'bz;``) and drivers of different bits of one net.
``tri0`` and ``tri1`` nets are pulled to ``0`` and ``1`` when nothing drives them, ``supply0`` and
``supply1`` nets are constant, and a ``uwire`` with two drivers on one bit is ``E4502``. A drive strength on a
continuous assignment or a net declaration assignment is read on three levels: supply and strong, pull and
weak (large, medium and small too), and ``highz``. A stronger driver wins; supply against strong, and pull
against weak, resolve to ``x`` where the standard picks the stronger value. VHDL sees the resolved
``std_ulogic``: a weak or pulled output shows ``L``, ``H`` or ``W``, and a ``bit`` port takes them as ``0``
and ``1``. An input port that the module also drives is ``E4501``.

**Not yet.** Module instances inside a Verilog module, generate constructs, gate primitives and
user-defined primitives, functions and tasks, memories, ``real`` variables, ``inout`` ports, wired nets
(``wand``, ``wor``, ``triand``, ``trior``), ``trireg``, ``pullup`` and ``pulldown``, delays on
continuous assignments and nets, ``defparam``, named events, ``fork``/``join``, ``disable``, procedural
``assign`` and ``force``, hierarchical names, the other system tasks, and the ``glbl`` module of vendor
libraries. Each stops elaboration with ``E4501``, naming the construct. Specify blocks are read and not
evaluated. Verilog runs in the interpreter only, and Verilog nets and variables other than the ports are
not in the wave file.

Diagnostics
-----------

Verilog diagnostics have codes ``E4xxx`` and ``W4xxx``, listed in :doc:`error-codes`. A Verilog module
as the top level is ``E4500``.
