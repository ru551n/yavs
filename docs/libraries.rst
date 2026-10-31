OSVVM, UVVM, VUnit and other libraries
======================================

This page says how to use a verification library or any other library of VHDL sources with yavs.

yavs is compliant with OSVVM, UVVM and VUnit: their own regression and test suites run with yavs,
and so do the testbenches of other open-source VHDL libraries. The current numbers
are in :doc:`conformance`.

yavs has no precompiled libraries and no installer command. Using a library means analyzing its
VHDL sources into a library of the name it expects.

.. contents:: On this page
   :local:

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

A test runner drives yavs through the three commands ``analyze``, ``elab`` and ``run``. VUnit's test suites run
with yavs. To drive yavs from your own scripts, see "Scripting" in :doc:`cli`.

Using yavs from VUnit
---------------------

The ``vunit-yavs`` package adds yavs to VUnit as a simulator: :doc:`vunit`.

Native libraries and Python from VHDL
-------------------------------------

Moved to :doc:`vunit`.

.. _vunit-token-server:

Sharing CPU between parallel VUnit tests
----------------------------------------

Moved to :doc:`threads`.

See also
--------

* :doc:`vunit`: using yavs from VUnit.
* :doc:`mixing-standards`: a library used under several standards.
* :doc:`conformance`: how the libraries' own test suites do.
