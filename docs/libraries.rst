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

Make VUnit select yavs as described in the
`package README <https://github.com/ru551n/yavs/blob/main/python/vunit-yavs/README.md>`__;
if another simulator is installed, VUnit may otherwise choose that one.

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

It helps when the long tests are large designs that split across threads, such as many parallel
processes on one clock. Small testbench-style tests and processor models keep their work on one
thread, so for them it gains nothing and costs nothing. Measurements are in
`parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__.

Vendor libraries
----------------

Vendor simulation libraries are not shipped, because the vendor licenses their sources. Analyze
them from your local installation, in the order the vendor documents. They run for functional
simulation: timing models run with zero delay and no SDF file is read.

For ``unisim``, analyze the component package before the designs that use it, and the primitives
the design instantiates (without them the instance stays unbound, warning ``W2108``, and its outputs
stay ``'U'``). With ``VENDOR_SIM_DIR`` naming the directory that holds ``unisims/``:

.. code-block:: bash

    U="$VENDOR_SIM_DIR/unisims"
    yavs analyze --std=2008 --work=unisim:build/unisim \
        "$U/unisim_VPKG.vhd" "$U/unisim_retarget_VCOMP.vhd" "$U/primitive/BUFG.vhd"
    yavs analyze --std=2008 --work=work:build/work --map=unisim:build/unisim my_design.vhd
    yavs run --work=work:build/work --map=unisim:build/unisim my_design

The VITAL and Synopsys packages these models use are built into yavs.
