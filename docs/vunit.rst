Using yavs from VUnit
=====================

This page goes from installing the ``vunit-yavs`` package to running tests, setting options, debugging a
test, collecting coverage and sharing CPU cores. :doc:`vunit-yavs` is the reference of every option.

.. contents:: On this page
   :local:

Install
-------

``vunit-yavs`` adds yavs to VUnit as a simulator. You need yavs 0.11.0 or later on ``PATH`` (:doc:`install`):

.. code-block:: bash

    pip install vunit-yavs

This also installs VUnit 5.0.0.dev16 or later, a pre-release, which ``pip`` picks up for this package. If yavs
is not on ``PATH``, set ``VUNIT_YAVS_PATH`` to its directory.

Features described as newer than 0.11.0 on this page need a release of both yavs and ``vunit-yavs`` after that
version.

A run script
------------

The files are in ``docs/examples/vunit`` of the source tree. They test the counter of the
:doc:`project walkthrough <project-walkthrough>`:

.. literalinclude:: examples/vunit/run.py
   :language: python
   :caption: run.py

.. literalinclude:: examples/vunit/tb_counter.vhd
   :language: vhdl
   :caption: tb_counter.vhd

Add the package before anything else in the script: it provides the simulator. Select yavs with the
``VUNIT_SIMULATOR`` environment variable, as this script does for itself. Without it VUnit picks the first
simulator it finds on ``PATH``, which may be another one.

Run tests
---------

VUnit works out the file order, analyzes what changed and runs the tests:

.. code-block:: bash

    python run.py                 # every test, one at a time
    python run.py -p 4            # four tests side by side
    python run.py --list          # the names of the tests
    python run.py 'tb.tb_counter.width_3.*'  # some of them
    python run.py -v              # also show the output of passing tests

.. code-block:: text

    pass tb.tb_counter.width_3.counts (0.7 s)
    pass tb.tb_counter.width_5.counts (1.0 s)
    pass tb.tb_counter.width_3.wraps  (0.5 s)
    pass tb.tb_counter.width_5.wraps  (0.2 s)
    =====================================================
    pass 4 of 4

Everything goes under ``vunit_out`` in the current directory, and ``-o DIR`` moves it. The libraries are in
``vunit_out/yavs/libraries`` (:doc:`on-disk`). The exit status is 0 only when every test passed.

Pass options to yavs
--------------------

Options of ``yavs`` commands go in sim and compile options of the package, on the project, a library, a
testbench or a test. Each flag is one list element, written ``--name=value``:

.. code-block:: python

    # Verification code with non-protected shared variables: analyze and run with --relaxed
    vu.set_compile_option("yavs.global_flags", ["--relaxed"])
    vu.set_sim_option("yavs.global_flags", ["--relaxed"])

    # Only for yavs run
    tb_counter.set_sim_option("yavs.sim_flags", ["--wave=counter.fst"])

:doc:`vunit-yavs` lists the options. Generics go through VUnit's own ``set_generic`` and ``add_config``
(:doc:`generics`), and the standard per file through ``vhdl_standard`` (:doc:`mixing-standards`).

Debug one test
--------------

``--yavs-debug`` runs the one selected test under the debugger and waits for a client. It prints where:

.. code-block:: text

    $ python run.py tb.tb_counter.width_3.counts --yavs-debug
    Debugging tb.tb_counter.width_3.counts: attach a DAP client to 127.0.0.1:4711

In VS Code, with the yavs extension installed, set your breakpoints and press F5, or run "yavs: Attach"
(:doc:`debugging`). Write another port as ``--yavs-debug=PORT``, and ``0`` picks a free one. A test name after a
bare ``--yavs-debug`` would be read as the port, so put the name first.

Newer than 0.11.0, there are two more ways to look at one test:

* ``--yavs-interactive`` runs it at the yavs console, with ``vunit_help`` and the other ``vunit_*`` commands
  (:doc:`interactive`).
* ``--yavs-surfer`` shows its waves in Surfer while it runs (:doc:`waveforms`).

Collect coverage
----------------

Newer than 0.11.0, the package collects, merges and checks coverage:

.. code-block:: bash

    python run.py --yavs-cov --yavs-cov-html cov-html --yavs-cov-fail-under 85

``--yavs-cov`` makes every simulation write a database. To have them merged, the run script calls
``results.merge_coverage`` after the run, as the script above does. It writes ``coverage.ycov``, and with
the other two flags also the HTML report and the gate. ``merge_coverage`` is an error when no simulation
collected coverage, so the script calls it only under ``--yavs-cov``.

The total counts everything that was simulated, also VUnit's own packages, which pull it far down. That is why
the script passes an exclusion file for them to ``yavs cov report``:

.. literalinclude:: examples/vunit/coverage-exclude.txt
   :language: text
   :caption: coverage-exclude.txt

A sim option collects coverage for some tests, or for every run:

.. code-block:: python

    vu.set_sim_option("enable_coverage", True)
    vu.set_sim_option("yavs.cov_kind", ["statement", "branch"])

Below the limit the run exits with status 1, after VUnit has written its reports. Coverage cannot be combined
with ``--yavs-debug``. :ref:`vunit-coverage` has more, and :doc:`coverage` the reports and exclusions.

Share CPU cores
---------------

``-p N`` runs N tests side by side and is usually all you need. Near the end of a regression, when only long
tests are left, the token server can lend the idle slots to them as threads:

.. code-block:: bash

    python run.py -p 8 --yavs-jobserver --yavs-max-threads 4

It helps only when those long tests gain from threads. :doc:`threads` says how to tell.

Other libraries in a test
-------------------------

Add them as in any VUnit script. VUnit's own libraries come from ``add_vhdl_builtins``, and OSVVM comes from
the ``vunit-osvvm`` package: ``pip install vunit-osvvm``, then ``vu.add_package("vunit-osvvm")``. Vendor libraries are in :doc:`vendor-libraries`.

.. _vunit-native-library:

Native libraries and Python from VHDL
-------------------------------------

A VUnit package or a project can call a native library from VHDL through foreign subprograms
(``attribute foreign of f : function is "VHPIDIRECT f"``), and a native library can embed Python,
so a testbench calls Python (a golden model, a file reader) from VHDL. yavs binds the subprograms
to the libraries named by ``--load=LIB`` (:doc:`cli-reference`). ``vunit-yavs`` knows this: the
simulator interface has ``foreign_language_interface = "VHPIDIRECT"`` and
``native_library_hooks(library_file)``, which returns the simulator hooks that pass
``--load=library_file`` to every ``yavs run``. A package registers them from its setup function:

.. code-block:: python

    def setup(context):
        library = build_native_library(context.output_path / "foo")
        context.register_simulator_hooks("yavs", **YavsInterface.native_library_hooks(library))

The repository holds a working example, ``python/vunit-yavs/tests/project_foreign``: a package that
compiles a C file (with the Python of the run script embedded where its headers and shared library
exist) and a testbench that calls ``native_add1`` and evaluates Python expressions from VHDL.

Text crosses the boundary as the characters of the VHDL string, one byte each. yavs reads a UTF-8
source file as ISO-8859-1 characters (``"é"`` is one character), so a literal with an accent reaches
the library as one byte that is not valid UTF-8. Give ``--encoding=latin1`` to ``yavs.a_flags`` and to
``yavs.global_flags`` as a sim option when the library expects the UTF-8 bytes of the file.

See also
--------

* :doc:`vunit-yavs`: every option and flag of the package.
* :doc:`debugging`: what the debugger can do.
* :doc:`threads`: threads and the token server.
* :doc:`ci`: VUnit in continuous integration.
