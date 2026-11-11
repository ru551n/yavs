What yavs stores on disk
========================

This page lists the files and directories yavs writes, what each is for, and how to clear them. All of
them can be deleted: the worst result is that something has to be analyzed again.

.. contents:: On this page
   :local:

A library directory
-------------------

The directory of ``--work=NAME:PATH`` or ``--map=NAME:PATH``:

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Name
     - What it is
   * - ``yavs-lib.toml``
     - The manifest: the files of the library, with path, hash, standard and design units.
   * - ``yavs-lib.d/``
     - One record per analyzed file.
   * - ``_YAVS_LIB``
     - The lock file. ``analyze`` holds it while it updates the library, so parallel calls are safe.
   * - ``yavs-cache/``
     - The compile cache of this library (next section).
   * - ``yavs-wave-layout/TOP.txt``
     - The signal list of the waveform viewer for testbench ``TOP`` (:doc:`waveforms`): one name per line. Deleting it brings back the default list.

A library holds no copy of your sources: it records their absolute paths and hashes, and ``run`` reads
them again. So the sources must stay where they were analyzed (``L0033`` if not) and unchanged (``L0034``).
A file has one entry however it is spelled (``..``, a symlink, another name of the same file): analyzing
it under a second spelling replaces the first entry. A library written by a yavs with another library
format (this one is 5) is refused with ``L0004``: delete the directory and analyze again.

The compile cache
-----------------

``yavs-cache/`` keeps what can be reused: analyzed design units, the lowered form of a design and the
compiled code. The first run of a design notes it, the second stores it, later runs reuse it
(:doc:`running-faster`); compiled code is stored from the first run, so the second takes what the first
compiled without making it again. Analyzed design units are stored by ``analyze``, and by a run that had to
analyze a unit again (for example an architecture analyzed before the entity it instantiates), so the
next testbench reads it instead.

* The key of an entry holds the yavs build and the options, so an edit or another yavs makes a new
  entry. Nothing has to be cleaned by hand.
* A library directory keeps at most about 4.75 GiB of cache: 4 GiB of design entries, 256 MiB of compiled
  code, 512 MiB of analyzed units. A suite only takes what it uses, about 10 MB for each testbench that runs
  more than once, so a suite of 120 testbenches in one library uses a little over 1 GB. Parallel runs can
  pass the limit by what they write in about 5 seconds.
* At the limit yavs first removes entries not used for an hour, and then the entries of the design that ran
  last, not the oldest ones. A suite larger than the limit therefore keeps the part that fits warm, and runs
  the rest without reuse. Measured with the 4 GiB limit: at about 2.4 times the limit later runs took 3 percent fewer
  instructions than the first, at about 3.6 times 1 percent more, at about 6 times 7 to 8 percent more. So
  **a suite more than about three times over the limit is slower warm than cold.** Larger suites were not run.
  The reason is the order of the tests: a test runner that orders later runs by earlier run times separates the
  test cases of a testbench, so what a design that does not fit stored is gone before its next test case. Split such a
  suite into several libraries, each with its own limit.
* Running an unchanged suite again reuses what is stored. Compiled code keeps growing for some runs, by less
  each time, as functions become hot enough to compile for the first time; measured on a suite of 120 testbenches
  it passed 40 MB after eight runs and then grew under 1 MB per run, and it did not reach zero.
* An entry that is damaged counts as not there. The cache can make a run faster, never wrong: a failure to
  read or write it is not an error.
* ``--no-cache`` turns it off for one command.

The shared store of std and ieee
--------------------------------

The analyzed ``std`` and ``ieee`` packages are kept once per user, not in each library:

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - System
     - Default directory
   * - Linux
     - ``$XDG_CACHE_HOME/yavs/std``, else ``$HOME/.cache/yavs/std``
   * - Windows
     - ``%LOCALAPPDATA%\yavs\std``

``--std-cache=DIR`` or the environment variable ``YAVS_STD_CACHE=DIR`` moves it, and ``off`` keeps these
units in each library's own cache instead. The option wins over the variable. A store that cannot be used is
error ``E0126``, on a machine with no cache directory for example: name a directory, or say ``off``.

Other files a run writes
------------------------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - File
     - When
   * - The file of ``--wave=FILE``
     - Written in FST format, whatever its name.
   * - ``<top>.fst`` beside the library directory
     - ``--surfer`` without ``--wave``; its path is printed.
   * - ``yavs.ycov`` in the current directory
     - ``--cov=LETTERS`` without ``--cov-file``.
   * - The file of ``--profile=FILE``
     - The profile as JSON.
   * - The file of ``--activity=FILE``
     - The activity tables as JSON.
   * - ``yavs/`` in VUnit's output directory
     - With ``vunit-yavs``: ``libraries/`` holds the analyzed libraries, ``std_cache/`` the ``std`` and
       ``ieee`` store of this output directory. ``test_output/`` holds the rest of each test.

How to clear them
-----------------

.. code-block:: bash

    # The cache of a library, and the shared store of std and ieee
    yavs cache clean --work=work:build/work

    # The same for several libraries, without the shared store
    yavs cache clean --work=rtl:build/rtl --work=work:build/work --std-cache=off

    # Everything: the library has to be analyzed again
    rm -rf build

``yavs cache clean`` prints what it removed:

.. code-block:: text

    build/rtl: removed 6 entries (7136 bytes)
    build/work: removed 4 entries (583951 bytes)

To remove yavs from a machine, delete its directory and ``yavs/std`` in your cache directory.

Which directories to cache in CI is in :doc:`ci`.

See also
--------

* :doc:`running-faster`: what the cache gains.
* :doc:`environment`: ``YAVS_STD_CACHE`` and the variables behind it.
* :doc:`ci`: caching between CI runs.
