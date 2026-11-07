Verilog
=======

yavs reads Verilog (IEEE 1364-2005) next to VHDL. The aim is to instantiate readable vendor Verilog,
such as primitive libraries and IP models, from a VHDL testbench.

.. note::

   This is under construction. Today ``yavs analyze`` reads Verilog files and records their modules
   in a library. Elaborating and simulating a design that instantiates a Verilog module is not
   implemented yet, and a closure that reaches a Verilog module stops with an error. SystemVerilog is
   not read.

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
IEEE 1364-2005, which has no elaboration tasks; vendor and cell models use them. They are recorded
and not executed yet: elaboration of Verilog is not implemented. Any other system task outside a
procedural block is still a syntax error (``E4100``).

What is checked
---------------

The analyzer checks the rules of IEEE 1364-2005 that need no elaboration, and the repository keeps one test per rule, with
its clause. Numbers must be well formed (``E4014``): a based constant has a non-zero size, at least one digit, no leading
underscore, and only digits of its base. Compiler directives must be complete and balanced (``E4004`` to ``E4011``). Gates have
the terminals their kind takes, user-defined primitives have one output, first, and at most one edge per table row, a path
delay has 1, 2, 3, 6 or 12 values, an input or inout port is not a ``reg`` (``E4202``), a ``case`` has one ``default``, a function has input
arguments only, and the variable of a generate ``for`` loop is a ``genvar`` (``E4204``); these are syntax errors
(``E4100``) unless a code is given. Some rules are left to elaboration, for example the name of an instance of a module
(only gates and primitives may omit it) and duplicate names inside a task or function.

Diagnostics
-----------

Verilog diagnostics have codes ``E4xxx`` and ``W4xxx``, listed in :doc:`error-codes`. ``elab`` and
``run`` refuse a library whose Verilog file, or a file it includes, changed since it was analyzed
(``L0034``, like a changed VHDL source): analyze the Verilog files again.
