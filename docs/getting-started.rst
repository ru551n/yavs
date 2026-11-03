Getting started
===============

This page says how to get yavs, a release archive for your platform or a build from source, and then takes you through your
first simulation, a failing check and an error in the VHDL. yavs works in two steps: ``analyze`` checks VHDL files and stores
them in a library, ``run`` elaborates a top level and simulates it.

.. contents:: On this page
   :local:

.. _installation:

Installation
------------

Releases
~~~~~~~~

Download the archive for your platform from the `releases
<https://github.com/ru551n/yavs/releases>`__ page (releases are marked as pre-releases while yavs is
early), check it against ``SHA256SUMS``, unpack it and put the directory it creates on ``PATH``:

* ``yavs-<version>-x86_64-unknown-linux-gnu.tar.gz``
* ``yavs-<version>-aarch64-unknown-linux-gnu.tar.gz``
* ``yavs-<version>-x86_64-pc-windows-msvc.zip``

aarch64 is also known as arm64; run ``uname -m`` to see which one you need. There is no macOS
build and no 32-bit build.

.. code-block:: bash

    sha256sum -c SHA256SUMS --ignore-missing     # Linux: checks the archives you downloaded
    tar xzf yavs-<version>-x86_64-unknown-linux-gnu.tar.gz
    export PATH="$PWD/yavs-<version>-x86_64-unknown-linux-gnu:$PATH"
    yavs --version

On Windows, compare ``(Get-FileHash yavs-<version>-x86_64-pc-windows-msvc.zip).Hash`` in
PowerShell with the line of ``SHA256SUMS``, unzip the archive and add its directory to ``PATH``.
``yavs --version`` prints the version and the kind of archive on the first line, for example
``yavs 0.12.0 (compiled tier: cranelift)``, and the copyright notice on the second.

Every platform has one archive with everything in it: nothing else needs to be installed. Its
``yavs`` has the compiled tier built in (:doc:`running-faster`). A release also carries the VS Code
extension of the same version (:doc:`debugging`), and ``pip install vunit-yavs`` gets the VUnit package (:doc:`vunit`).

What is in the archive
^^^^^^^^^^^^^^^^^^^^^^

Each archive unpacks into one directory of the same name (without ``.tar.gz`` or ``.zip``):

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - File
     - What it is
   * - ``yavs``, ``yavs.exe``
     - The simulator. The ``std`` and ``ieee`` sources are built into it.
   * - ``LICENSE-MIT``, ``LICENSE-APACHE``
     - The licences of yavs, MIT or Apache-2.0 at your option.
   * - ``LICENSE-IEEE``, ``LICENSE-IEEE-VITAL``, ``LICENSE-SYNOPSYS``
     - The licence and notices of the built-in ``std`` and ``ieee`` sources, the VITAL packages and the
       Synopsys packages.
   * - ``README.md``
     - The project readme.

Requirements
^^^^^^^^^^^^

* The Linux archives require glibc 2.17 or newer (RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+). On
  musl-based systems such as Alpine, install the ``gcompat`` package. The archives are tested on
  x86_64 and aarch64 Linux and on Windows x86_64.
* ``yavs`` needs nothing else installed: no VHDL libraries, no Python.

Updating and removing
^^^^^^^^^^^^^^^^^^^^^

Unpack the new archive in a new directory and change ``PATH``. A library directory written by a yavs
whose library format differs is refused with error ``L0004`` naming the directory: delete it and
analyze its files again (libraries are not converted). The compilation cache of another yavs build is
ignored, never an error. To remove yavs, delete the directory; ``yavs cache clean --work=NAME:PATH``
removes what it cached in a library and in the shared ``std``/``ieee`` store.

From source
~~~~~~~~~~~

You need a recent stable Rust toolchain and a C compiler. Build with the Cranelift tier, as the
release archives are:

.. code-block:: bash

    git clone https://github.com/ru551n/yavs
    cd yavs
    cargo build --release -p yavs --features clift   # target/release/yavs

Without ``--features clift`` the build has no compiled tier: ``yavs --version`` says
``interpreter only, no compiled tier``, the default is ``--tier=interp``, and ``--tier=auto`` is
error ``E0120``.

Profile-guided build
^^^^^^^^^^^^^^^^^^^^

A build can be optimised with a profile of yavs's own run time. ``tools/pgo/build.sh`` builds
yavs with instrumentation, runs a set of bundled testbenches with it, and builds again using what
the runs recorded. It needs the ``llvm-tools`` component of the Rust toolchain, a ``bash`` and ``python3``:

.. code-block:: bash

    rustup component add llvm-tools
    tools/pgo/build.sh target/yavs-pgo     # the profile-guided yavs

It takes about three times the processor time of a plain release build (two builds and the training runs, about
ten processor minutes of them) and the result gives the same output as a plain build. On the real testbenches we
measured it executes about 5 percent fewer instructions and finishes in roughly 10 percent less time; see :doc:`performance`.
Set ``RUSTFLAGS`` (for example ``-C target-cpu=x86-64-v3``) to give both builds the same flags. The release
archives are not built this way, and the build is tested on Linux x86-64 only.

Problems with the installation are covered in :doc:`troubleshooting`.

.. _quickstart:

Your first simulation
---------------------

You need ``yavs`` on ``PATH`` and the file ``hello_tb.vhd`` (it is in
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
        Process hello_tb.check
    ** Note: 32000000fs+0: done
        Process hello_tb.check
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
        Process hello_tb.check
    ** Failure: 32000000fs+0: expected 5, got 3
        Process hello_tb.check

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

Analyze files in dependency order, packages before the units that use them. A library other than
``--work`` is mapped with ``--map=NAME:PATH``:

.. code-block:: bash

    yavs analyze --work=mylib:build/mylib src/pkg.vhd src/dut.vhd
    yavs analyze --work=work:build/work --map=mylib:build/mylib tb/tb.vhd
    yavs run --work=work:build/work --map=mylib:build/mylib tb --stop-time=10us

* After you edit a file, analyze it again before ``run``: ``run`` stops with ``L0034`` for a file
  that changed since it was analyzed.
* You do not have to analyze what depends on it: ``run`` reads the files of the design again, so
  every unit is checked against the current version of what it uses (:doc:`migrating`).
* ``std`` and ``ieee`` (with ``numeric_std``, ``fixed_pkg``, ``float_pkg``, the VITAL and the
  Synopsys packages) are built in; there is nothing to compile for them.
* A testbench with a free-running clock and no ``finish`` or ``stop`` never ends by itself: add
  ``--stop-time=1us`` or press Ctrl-C.

:doc:`project-walkthrough` goes through all of this with a small project.

See also
--------

* :doc:`project-walkthrough`: several libraries, rebuilds, generics, waves and coverage.
* :doc:`on-disk`: what yavs keeps on disk.
* :doc:`migrating`: what is different if you know another simulator.
* :doc:`vunit`: install the VUnit package and run tests with it.
* :doc:`interactive`: a debugger at a terminal prompt, and :doc:`debugger-tutorial` for a walk through a failing test.
* :doc:`debugging`: breakpoints in VS Code.
* :doc:`lint`: find what is provably wrong before a simulation.
* :doc:`cli-reference`: every option in one place.
