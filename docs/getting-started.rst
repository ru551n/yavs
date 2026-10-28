Getting started
===============

yavs works in two steps: ``analyze`` checks VHDL files and stores them in a library, ``run``
elaborates a top level and simulates it.

.. _quickstart:

Your first simulation
---------------------

You need ``yavs`` on ``PATH`` (:doc:`install`) and the file ``hello_tb.vhd`` (it is in
``docs/examples`` of the source tree). The testbench counts clock edges, reports the count after
32 ns, checks it against the generic ``EXPECTED`` and ends the run with ``finish``:

.. literalinclude:: examples/hello_tb.vhd
   :language: vhdl
   :caption: hello_tb.vhd

Analyze the file into a library, then run the top level:

.. code-block:: bash

    yavs analyze --work=work:build/work hello_tb.vhd
    yavs run --work=work:build/work hello_tb

``--work=work:build/work`` is the library named ``work``, kept in the directory ``build/work``
(``analyze`` creates it). ``analyze`` prints nothing when it succeeds. ``run`` prints the reports
of the simulation:

.. code-block:: text

    ** Note: 32000000fs+0: hello, count is 3
        Process :hello_tb:check
    ** Note: 32000000fs+0: done
        Process :hello_tb:check
    ** Note: 32000000fs+0: FINISH called with status 0

Each report shows its severity, the simulation time in femtoseconds (``32000000fs`` is 32 ns), the
delta cycle after the plus sign, and the text; the next line is the process that reported it. The
exit status is 0. The top level is the entity name, or ``hello_tb.sim`` to pick the architecture.

A failing check
---------------

Override the generic so that the assertion fails:

.. code-block:: bash

    yavs run --work=work:build/work hello_tb -gEXPECTED=5

.. code-block:: text

    ** Note: 32000000fs+0: hello, count is 3
        Process :hello_tb:check
    ** Failure: 32000000fs+0: expected 5, got 3
        Process :hello_tb:check

A ``failure`` stops the simulation and the exit status is 1, so a script or test runner sees the
test as failed. An ``error`` report does not stop the run, but the exit status is still 1 at the
end; ``--exit-severity=error`` makes it stop too.

An error in the VHDL
--------------------

``hello_error.vhd`` (also in ``docs/examples``) uses a name that does not exist:

.. literalinclude:: examples/hello_error.vhd
   :language: vhdl
   :caption: hello_error.vhd

.. code-block:: text

    $ yavs analyze --work=work:build/work hello_error.vhd
    error[E1100]: `cnt` is not declared
     --> hello_error.vhd:7:12
      |
    5 |   signal count : natural;
      |          ----- similar name declared here
    6 | begin
    7 |   count <= cnt + 1;
      |            ^^^ not found in this scope
      |
      = help: a signal with a similar name exists: `count`

    error: aborting due to 1 previous error

``yavs --explain E1100`` explains the code. yavs stops at the first failing step and never runs a
design after an error; :doc:`troubleshooting` has more.

A project with several files and libraries
------------------------------------------

Analyze files in dependency order, packages before the units that use them. Several files can go
in one call; a library other than ``--work`` is mapped with ``--map=NAME:PATH``:

.. code-block:: bash

    yavs analyze --work=mylib:build/mylib src/pkg.vhd src/dut.vhd
    yavs analyze --work=work:build/work --map=mylib:build/mylib tb/tb.vhd
    yavs run --work=work:build/work --map=mylib:build/mylib tb --stop-time=10us

* ``--std=93``, ``--std=2008`` (the default) or ``--std=2019`` sets the standard of the files of
  one ``analyze`` call. A project can mix standards: call ``analyze`` once per standard; ``run``
  needs no ``--std``.
* After you edit a file, analyze it again before ``run``: ``run`` stops with ``L0034`` for a file
  that changed since it was analyzed.
* ``std`` and ``ieee`` (with ``numeric_std``, ``fixed_pkg``, ``float_pkg``, the VITAL and the
  Synopsys packages) are built in; there is nothing to compile for them.
* A testbench with a free-running clock and no ``finish`` or ``stop`` never ends by itself: add
  ``--stop-time=1us`` or press Ctrl-C.

Next steps
----------

* Record waveforms with ``--wave=tb.fst``: :doc:`waveforms`.
* Use OSVVM, UVVM, VUnit or vendor libraries: :doc:`libraries`.
* Set breakpoints in VS Code: :doc:`debugging`.
* Every option in one place: :doc:`cli`.
