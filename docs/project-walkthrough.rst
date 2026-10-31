A real project, step by step
============================

This page takes a small project from its source files to waveforms and coverage: several libraries,
file order, rebuilding after an edit, generics and a run script. Read :doc:`getting-started` first.

.. contents:: On this page
   :local:

The project
-----------

The files are in ``docs/examples/project`` of the source tree. Copy that directory and work in it.

.. code-block:: text

    project/
      rtl/counter_pkg.vhd   library rtl: a function the design uses
      rtl/counter.vhd       library rtl: a counter with a generic width
      tb/counter_tb.vhd     library work: the testbench, which has generics of its own
      run.sh                the commands of this page as a script

.. literalinclude:: examples/project/rtl/counter.vhd
   :language: vhdl
   :caption: rtl/counter.vhd

.. literalinclude:: examples/project/tb/counter_tb.vhd
   :language: vhdl
   :caption: tb/counter_tb.vhd

Analyze the libraries
---------------------

A library is a directory. ``--work=NAME:PATH`` names the library the files go into, and creates the
directory when it does not exist yet. Files are analyzed in the order you give them, so a package
comes before the units that use it:

.. code-block:: bash

    yavs analyze --work=rtl:build/rtl rtl/counter_pkg.vhd rtl/counter.vhd

yavs does not sort files for you. In the wrong order the first file stops with ``E1002``:

.. code-block:: text

    $ yavs analyze --work=rtl:build/rtl rtl/counter.vhd rtl/counter_pkg.vhd
    error[E1002]: design unit `counter_pkg` not found in library `rtl`
     --> rtl/counter.vhd:5:10
      |
    5 | use work.counter_pkg.all;
      |          ^^^^^^^^^^^ needed here

Inside ``rtl/counter.vhd``, ``work`` means the library the file is analyzed into, here ``rtl``.

The testbench goes into a library of its own. ``--map=NAME:PATH`` makes another library visible; repeat
it for each library the code names:

.. code-block:: bash

    yavs analyze --work=work:build/work --map=rtl:build/rtl tb/counter_tb.vhd

Without the ``--map`` the testbench stops with ``E1001``, ``unknown library `rtl```.

Run the testbench
-----------------

``run`` needs the same mappings as ``analyze``. The last word is the top level, in the ``--work``
library:

.. code-block:: bash

    yavs run --work=work:build/work --map=rtl:build/rtl counter_tb

.. code-block:: text

    ** Note: 335000000fs+0: wrapped 2 times at 335000000 fs
        Process :counter_tb:main
    ** Note: 335000000fs+0: FINISH called with status 0

The exit status is 0.

Change the generics
-------------------

``-gNAME=VALUE`` sets a generic of the top level, for one run. Nothing is analyzed again:

.. code-block:: bash

    yavs run --work=work:build/work --map=rtl:build/rtl counter_tb -gWIDTH=3
    yavs run --work=work:build/work --map=rtl:build/rtl counter_tb -gEXPECTED_WRAPS=3 -gCLOCK_PERIOD=2ns

:doc:`generics` has the forms of the values.

Rebuild after an edit
---------------------

``run`` checks that the sources are the ones it analyzed, by their contents. After an edit of
``rtl/counter_pkg.vhd`` it stops:

.. code-block:: text

    $ yavs run --work=work:build/work --map=rtl:build/rtl counter_tb
    error[L0034]: `/home/me/project/rtl/counter_pkg.vhd` changed on disk since it was analyzed into library `rtl`
      |
      = help: re-analyze it: `yavs analyze --std=2008 --work=rtl:build/rtl /home/me/project/rtl/counter_pkg.vhd`

There are two ways to rebuild:

* Analyze the edited file alone. ``run`` then also checks the units that depend on it: if the edit
  removed something ``rtl/counter.vhd`` uses, ``run`` reports that file's error.
* Run all the ``analyze`` commands again. A file that did not change costs almost nothing, so a script
  can do this every time. This is the simple choice, and the one below.

The run script
--------------

.. literalinclude:: examples/project/run.sh
   :language: bash
   :caption: run.sh

``set -e`` stops the script at the first failing step. The arguments go on to ``yavs run``, so the
later sections pass their options through it:

.. code-block:: bash

    ./run.sh -gWIDTH=3

On Windows, run the same three commands from a script of your own. A long list of files can go in a
response file, one argument per line, given as ``@FILE`` (:doc:`cli`).

Record waveforms
----------------

.. code-block:: bash

    ./run.sh --wave=build/tb.fst

Open ``build/tb.fst`` in a waveform viewer that reads FST. :doc:`waveforms` says how to choose what is
recorded and how to watch a run while it goes.

Measure coverage
----------------

Run with ``--cov``, then report on the database it writes:

.. code-block:: bash

    ./run.sh --cov=build/tb.ycov --cov-kind=statement,branch
    yavs cov report build/tb.ycov

.. code-block:: text

    statements 19/19 100.0%
    branches   8/8 100.0%
    toggles    0/0 -

    path             statements            branches              toggles
    :counter_tb      10/10 100.0%          3/3 100.0%            0/0 -
    :counter_tb:dut  8/8 100.0%            5/5 100.0%            0/0 -
    rtl.counter_pkg  1/1 100.0%            0/0 -                 0/0 -

:doc:`coverage` has the other kinds, HTML reports, merging runs and exclusions.

What to keep and what to ignore
-------------------------------

``build/`` holds only what yavs wrote, so it can be deleted at any time and does not belong in version
control. :doc:`on-disk` lists what is in it.

See also
--------

* :doc:`libraries`: the same steps for OSVVM, UVVM and other libraries.
* :doc:`vunit`: let a test runner work out the file order and run many tests.
* :doc:`mixing-standards`: files of different VHDL standards in one project.
* :doc:`ci`: run this in continuous integration.
