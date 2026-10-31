Mixing language standards
=========================

This page shows how to use files of different VHDL standards in one project: old code next to new
code, without changing either.

.. contents:: On this page
   :local:

How it works
------------

``--std`` on ``analyze`` is the standard of the files of that call. yavs records the standard per file,
so ``run`` and ``elab`` need no ``--std``.

The files in ``docs/examples/mixed`` show it. The package uses ``default`` as a name, which is allowed in
VHDL-1993 and reserved in VHDL-2008. The testbench uses ``process (all)``, which VHDL-1993 does not have:

.. literalinclude:: examples/mixed/old_pkg.vhd
   :language: vhdl
   :caption: old_pkg.vhd

.. literalinclude:: examples/mixed/mixed_tb.vhd
   :language: vhdl
   :caption: mixed_tb.vhd

Analyze once per standard, then run:

.. code-block:: bash

    yavs analyze --std=93 --work=work:build/work old_pkg.vhd
    yavs analyze --std=2008 --work=work:build/work mixed_tb.vhd
    yavs run --work=work:build/work mixed_tb

.. code-block:: text

    ** Note: 1000000fs+0: b is 4
        Process :mixed_tb:_p1
    ** Note: 1000000fs+0: FINISH called with status 0

Both files can be in one library, or in several. With the wrong standard a file stops at once:
``old_pkg.vhd`` under ``--std=2008`` is ``E0002``, ``mixed_tb.vhd`` under ``--std=93`` is ``E0011``.

Which packages a design sees
----------------------------

A design runs against one revision of ``std`` and ``ieee``: the newest among its files. In the example
that is VHDL-2008, also for ``old_pkg.vhd``. ``--std`` on ``run`` overrides the choice.

This has one consequence for old code. A VHDL-1993 file in a design that also has VHDL-2008 files is
checked as VHDL-1993 but sees the VHDL-2008 packages. ``std_logic_vector`` is then a subtype of
``std_ulogic_vector``, so code that overloads a subprogram on both fails with ``E1101``, a duplicate
declaration. Keep such code in a design of VHDL-1993 files alone.

A library used under several standards
--------------------------------------

A library is analyzed under the standard of each of its files, once. To use it under two standards, analyze
it twice, into two directories. Keep in mind that ``integer`` is 32 bits in VHDL-2008 and 64 bits in VHDL-2019
(:doc:`language-support`).

With VUnit
----------

Give the standard per file. In a VUnit run script ``add_source_files`` takes a ``vhdl_standard``
argument:

.. code-block:: python

    lib = vu.add_library("lib")
    lib.add_source_files("old_pkg.vhd", vhdl_standard="1993")
    lib.add_source_files("mixed_tb.vhd", vhdl_standard="2008")

See also
--------

* :doc:`language-support`: what each standard supports.
* :doc:`language-choices`: differences between simulators, including mixed standards.
* :doc:`migrating`: moving a project from another simulator.
