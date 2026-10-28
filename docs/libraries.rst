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
