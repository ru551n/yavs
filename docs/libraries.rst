OSVVM, UVVM, VUnit and other libraries
======================================

yavs is compliant with OSVVM, UVVM and VUnit: their own regression and test suites run with yavs,
and so do the testbenches of other open-source VHDL libraries. The current numbers
are in :doc:`conformance`.

yavs has no precompiled libraries and no installer command. Using a library means analyzing its
VHDL sources into a library of the name it expects.

Compile a library from source
-----------------------------

Analyze the sources in the compile order the library documents, into a library with the name its
users write in their ``library`` clauses, then map that library when you analyze and run your own
code:

.. code-block:: bash

    yavs analyze --std=2008 --work=osvvm:build/osvvm @osvvm-files.txt
    yavs analyze --std=2008 --work=work:build/work --map=osvvm:build/osvvm tb.vhd
    yavs run --work=work:build/work --map=osvvm:build/osvvm tb

``@osvvm-files.txt`` is a response file: one argument per line, here the source files in compile
order (:doc:`cli`). Analyze a library once; ``run`` only loads the units the top level reaches, so a
large library does not slow down each run.

* Analyze each library once per standard you use it with (``--std=2008`` or ``--std=2019``).
* Verification code sometimes uses constructs the standard forbids, such as a shared variable of a
  type that is not protected. ``--relaxed`` turns those few errors into warnings; pass it to
  ``analyze`` and to ``run``.
* ``run`` never writes to a library, so any number of simulations can run at once on the same
  libraries, and parallel ``analyze`` calls into one library are safe too.

Test runners
------------

A test runner drives yavs through the three commands ``analyze``, ``elab`` and ``run``. VUnit's
test suites run with yavs. To drive yavs from your own scripts, see "Scripting" in :doc:`cli`.

Using yavs from VUnit
---------------------

The ``vunit-yavs`` package adds yavs to VUnit as a simulator. It needs yavs 0.11.0 or later on
``PATH``. VUnit 5.0.0.dev16 or later is a pre-release, which ``pip`` picks up for this package:

.. code-block:: bash

    pip install vunit-yavs

Add the package in ``run.py`` before anything else; the rest of the script is a normal VUnit script:

.. code-block:: python

    from vunit import VUnit

    vu = VUnit.from_argv()
    vu.add_package("vunit-yavs", allow_setup=True)  # first: it provides the simulator
    vu.add_vhdl_builtins()
    vu.add_library("lib").add_source_files("*.vhd")
    vu.main()

Select yavs with the ``VUNIT_SIMULATOR`` environment variable; without it VUnit picks the first
simulator it finds on ``PATH``, which may be another one:

.. code-block:: bash

    VUNIT_SIMULATOR=yavs python run.py

or set it at the top of ``run.py``, before VUnit is created:

.. code-block:: python

    import os
    os.environ.setdefault("VUNIT_SIMULATOR", "yavs")

More options are in the
`package README <https://github.com/ru551n/yavs/blob/main/python/vunit-yavs/README.md>`__.

.. _vunit-token-server:

Sharing CPU between parallel VUnit tests
----------------------------------------

With ``-p N`` VUnit runs N tests side by side, and at the end of a run most of those N slots are
idle while a few long simulations finish alone. The token server lends the idle slots to the
simulations still running as extra threads, so the long tail ends sooner. A run never keeps more
than N threads busy once it has settled (a test that starts while every slot is lent starts at
once, so the total can briefly exceed N). The output is identical to a run without it. It works on
Linux and Windows.

It is part of the ``vunit-yavs`` package (``pip install vunit-yavs``, with yavs 0.11.0 or later).
It is off by default; turn it on with ``--yavs-jobserver`` together with ``-p`` above 1 (with
``-p 1`` there is nothing to lend and the flag does nothing):

.. code-block:: bash

    python run.py -p 8 --yavs-jobserver
    python run.py -p 8 --yavs-jobserver --yavs-max-threads 4

``--yavs-max-threads N`` caps the threads of one simulation (default: ``-p N``); a test can set its
own cap with the ``yavs.max_threads`` simulation option, e.g.
``vu.set_sim_option("yavs.max_threads", 2)``.

yavs splits the processes that wake in the same delta cycle across threads, and only when there is
enough work to share. It helps designs where many instances do real work on the same clock edge:

* generate loops of processing elements, multi-lane or multi-channel datapaths, MAC or systolic arrays;
* processes with heavy work per clock edge (wide arithmetic, vector loops, records);
* large flat RTL with many independent clocked processes.

It does not help (and today costs a few percent) when the long tests are:

* processor models or other designs where one or a few large processes do most of the work;
* testbench-style tests (a stimulus process, a checker and a small design);
* long chains of delta cycles that wake only a few processes each;
* processes that must run in order, such as file or text I/O and shared access or protected objects.

To decide, run the longest test of the regression alone with ``yavs run --threads=4`` and with
``--threads=1``. If it is at least about 1.3 times faster with 4, turn the token server on for that
regression, with a cap such as ``--yavs-max-threads 4``: most designs gain little beyond 4 threads.
Measurements are in `parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__.

Vendor libraries
----------------

Vendor simulation libraries are not shipped, because the vendor licenses their sources. Analyze
them from your own install like any other library. For a Vivado install, with ``S`` the directory
``<install>/data/vhdl/src``:

.. code-block:: bash

    S=$XILINX_VIVADO/data/vhdl/src   # the variable comes from the install's settings64.sh
    yavs analyze --work=unisim:build/unisim \
        $S/unisims/unisim_VPKG.vhd $S/unisims/unisim_retarget_VCOMP.vhd \
        $S/unisims/primitive/*.vhd $S/unisims/retarget/*.vhd
    yavs analyze --work=unimacro:build/unimacro --map=unisim:build/unisim \
        $S/unimacro/unimacro_VCOMP.vhd $S/unimacro/*.vhd
    yavs analyze --work=work:build/work --map=unisim:build/unisim my_design.vhd
    yavs run --work=work:build/work --map=unisim:build/unisim my_design

This takes between one and two seconds, and a fraction of a second when nothing changed, so a build
script can run it every time. On Windows, give the file list in a response file (:doc:`cli`).
Leave out ``unisim_VCOMP.vhd``, which defines the same package as ``unisim_retarget_VCOMP.vhd``.
A library records where its sources are and copies nothing, so the install must stay in place; after
it changes, analyze again (``L0034`` tells you).

With VUnit, the same with ``add_library``; VUnit works out the order:

.. code-block:: python

    S = Path(os.environ["XILINX_VIVADO"]) / "data" / "vhdl" / "src"
    unisim = vu.add_library("unisim")
    unisim.add_source_files([S / "unisims" / "unisim_VPKG.vhd", S / "unisims" / "unisim_retarget_VCOMP.vhd"])
    unisim.add_source_files(S / "unisims" / "primitive" / "*.vhd")
    unisim.add_source_files(S / "unisims" / "retarget" / "*.vhd")
    vu.add_library("unimacro").add_source_files(S / "unimacro" / "*.vhd")

The libraries run for functional simulation: timing models run with zero delay and no SDF file is
read. The VITAL and Synopsys packages these models use are built into yavs.

Buffers, registers, LUTs, shift registers, clocking (``MMCME2``, ``MMCME4``, ``PLLE2``, ``PLLE4``,
``BUFG`` and its relatives), block RAM (``RAMB18E1``, ``RAMB36E1``, ``RAMB18E2``, ``RAMB36E2``), DSP
slices (``DSP48E1``, ``DSP48E2``), hard FIFOs (``FIFO18E1``, ``FIFO36E1``, ``FIFO36E2``), the DDR
registers (``IDDR``, ``ODDR``, ``IDDRE1``, ``ODDRE1``), the delay elements and the ``unimacro`` macros
all run. Not available, because the install has them only as encrypted Verilog models or as
SystemVerilog:

* the primitives of ``unisims/secureip``: ``ISERDESE2``, ``ISERDESE3``, ``OSERDESE2``, ``OSERDESE3``,
  ``PHASER_*``, ``PHY_CONTROL``, ``IN_FIFO``, ``OUT_FIFO``, the bitslices, and the transceiver, PCIe,
  CMAC, Interlaken, HBM, RF converter and PS8 blocks. An instance of one stays unbound (warning
  ``W2108``) and its outputs stay ``'U'``: give it a behavioural model of your own for simulation.
* the XPM macros.
