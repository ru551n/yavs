Vendor libraries
================

This page says plainly which vendor simulation libraries work with yavs today, and how to use them. yavs
ships none, because the vendors license their sources: you analyze them from your own install, with plain
``analyze``. There is no special command.

.. contents:: On this page
   :local:

What works
----------

.. list-table::
   :header-rows: 1
   :widths: 40 15 45

   * - Library
     - State
     - Note
   * - Xilinx ``UNISIM``, ``UNIMACRO``, ``UNIFAST``
     - Works
     - Plain ``analyze``, functional simulation.
   * - Xilinx ``secureip`` primitives
     - Cannot work
     - Encrypted Verilog models. yavs does not decrypt protected IP, and has no Verilog.
   * - Xilinx XPM macros
     - Cannot work yet
     - SystemVerilog. Needs Verilog support, which is `planned for plain Verilog
       <https://github.com/ru551n/yavs/issues/776>`__.
   * - Intel and Lattice libraries
     - Not done
     - Nobody has done this work yet.

Timing models run with zero delay, and no SDF file is read. The VITAL and Synopsys packages these models use
are built into yavs.

Xilinx UNISIM, UNIMACRO and UNIFAST
-----------------------------------

For a Vivado install, ``XILINX_VIVADO`` comes from the install's ``settings64.sh``:

.. code-block:: bash

    S=$XILINX_VIVADO/data/vhdl/src
    yavs analyze --work=unisim:build/unisim \
        $S/unisims/unisim_VPKG.vhd $S/unisims/unisim_retarget_VCOMP.vhd \
        $S/unisims/primitive/*.vhd $S/unisims/retarget/*.vhd
    yavs analyze --work=unimacro:build/unimacro --map=unisim:build/unisim \
        $S/unimacro/unimacro_VCOMP.vhd $S/unimacro/*.vhd
    yavs analyze --work=work:build/work --map=unisim:build/unisim my_design.vhd
    yavs run --work=work:build/work --map=unisim:build/unisim my_design

* Leave out ``unisim_VCOMP.vhd``. It defines the same package as ``unisim_retarget_VCOMP.vhd``.
* This takes a second or so, and a fraction of that when nothing changed, so a build script can run it every
  time.
* A library records where its sources are and copies nothing, so the install must stay in place. After it
  changes, analyze again (``L0034`` tells you).
* On Windows, give the file list in a response file (:doc:`cli`).

``UNIFAST`` is analyzed the same way:

.. code-block:: bash

    yavs analyze --work=unifast:build/unifast --map=unisim:build/unisim $S/unifast/primitive/*.vhd

With VUnit, the same with ``add_library``. VUnit works out the order:

.. code-block:: python

    import os
    from pathlib import Path

    S = Path(os.environ["XILINX_VIVADO"]) / "data" / "vhdl" / "src"
    unisim = vu.add_library("unisim")
    unisim.add_source_files([S / "unisims" / "unisim_VPKG.vhd", S / "unisims" / "unisim_retarget_VCOMP.vhd"])
    unisim.add_source_files(S / "unisims" / "primitive" / "*.vhd")
    unisim.add_source_files(S / "unisims" / "retarget" / "*.vhd")
    vu.add_library("unimacro").add_source_files(S / "unimacro" / "*.vhd")

Primitives that run
~~~~~~~~~~~~~~~~~~~

Buffers, registers, LUTs, shift registers, clocking (``MMCME2``, ``MMCME4``, ``PLLE2``, ``PLLE4``, ``BUFG``
and its relatives), block RAM (``RAMB18E1``, ``RAMB36E1``, ``RAMB18E2``, ``RAMB36E2``), DSP slices
(``DSP48E1``, ``DSP48E2``), hard FIFOs (``FIFO18E1``, ``FIFO36E1``, ``FIFO36E2``), the DDR registers (``IDDR``,
``ODDR``, ``IDDRE1``, ``ODDRE1``), the delay elements and the ``unimacro`` macros.

What cannot work
----------------

The install has these only as encrypted Verilog models or as SystemVerilog:

* The primitives of ``unisims/secureip``: ``ISERDESE2``, ``ISERDESE3``, ``OSERDESE2``, ``OSERDESE3``,
  ``PHASER_*``, ``PHY_CONTROL``, ``IN_FIFO``, ``OUT_FIFO``, the bitslices, and the transceiver, PCIe, CMAC,
  Interlaken, HBM, RF converter and PS8 blocks. An instance of one stays unbound (warning ``W2108``) and its
  outputs stay ``'U'``: give it a behavioural model of your own for simulation.
* The XPM macros.

A file with an encrypted envelope stops with ``E0012``. yavs does not decrypt protected IP.

See also
--------

* :doc:`libraries`: verification libraries, analyzed the same way.
* :doc:`faq`: other limitations.
* :doc:`error-codes`: ``W2108`` and ``E0012``.
