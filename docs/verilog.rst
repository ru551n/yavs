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

SystemVerilog
-------------

SystemVerilog is not read. A ``.sv`` or ``.svh`` file is ``E4002``, and a SystemVerilog keyword in
a Verilog file is ``E4003``.

Diagnostics
-----------

Verilog diagnostics have codes ``E4xxx`` and ``W4xxx``, listed in :doc:`error-codes`. ``elab`` and
``run`` refuse a library whose Verilog file, or a file it includes, changed since it was analyzed
(``L0034``, like a changed VHDL source): analyze the Verilog files again.
