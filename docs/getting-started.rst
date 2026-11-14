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

.. _one-line-install:

One-line install
~~~~~~~~~~~~~~~~

On Linux (x86_64 or aarch64), one line installs yavs or updates it, without root:

.. code-block:: bash

    curl -fsSL https://raw.githubusercontent.com/ru551n/yavs/main/install.sh | sh

On Windows (x86_64), in PowerShell, with no administrator rights:

.. code-block:: powershell

    powershell -c "irm https://raw.githubusercontent.com/ru551n/yavs/main/install.ps1 | iex"

Piping a download into a shell runs whatever it fetched, so read it first if you like: both scripts are short
plain text, `install.sh <https://github.com/ru551n/yavs/blob/main/install.sh>`__ and `install.ps1
<https://github.com/ru551n/yavs/blob/main/install.ps1>`__. They are served from the ``main`` branch of the source
repository because every release is still marked as a pre-release, and GitHub's ``latest`` download address skips
those. All the logic is in one function that runs on the last line, so a download that stops early does nothing.

What they do, the same on both systems:

* They find the newest release (``--version 0.14.0`` on Linux, ``$env:YAVS_VERSION='0.14.0'`` on Windows installs that one;
  older versions are allowed and the script says it is a downgrade) and choose the archive. On x86_64 that is the plain
  archive if the CPU has AVX2 and the other features of x86-64-v3, and the ``-compat`` archive otherwise
  (:ref:`installation`). The script says which one and why. ``install.sh`` reads the feature list in ``/proc/cpuinfo``;
  ``install.ps1`` asks Windows for AVX2 (``IsProcessorFeaturePresent``), which every Intel and AMD CPU with AVX2
  has the other features with, and the downloaded ``yavs`` checks them all before it is installed. ``--compat``
  (Linux) or ``$env:YAVS_COMPAT=1`` (Windows) forces the ``-compat`` archive; it needs yavs 0.14.0 or newer.
* They download the archive over HTTPS and check it against the ``SHA256SUMS`` of the release. A mismatch stops the
  installation with nothing installed. That protects against a damaged download; it does not protect against a
  release that was itself replaced, because ``SHA256SUMS`` comes from the same place. On Linux without ``sha256sum`` or
  ``shasum`` the script refuses unless you pass ``--allow-unverified``.
* They run the new ``yavs --version`` before they put it in place, and refuse if it does not run or is not the version
  asked for.
* They install into your own directories: on Linux ``yavs`` in ``~/.local/bin`` and ``README.txt`` and the licences in
  ``~/.local/share/yavs`` (``--prefix DIR`` or ``YAVS_INSTALL_DIR`` chooses another prefix); on Windows everything in
  ``%LOCALAPPDATA%\Programs\yavs`` (``YAVS_INSTALL_DIR`` chooses another directory). To remove yavs, delete those files.

**Updating** is running the same line again. It says ``yavs 0.13.0 -> 0.14.0``, or ``already at 0.14.0`` and does
nothing (``--force`` on Linux, ``$env:YAVS_FORCE=1`` on Windows installs it again). The new binary replaces the old one
in a single rename, so a simulation that is running goes on with the old file, and an update that is interrupted leaves the
old ``yavs`` working. On Windows a running ``yavs.exe`` cannot be overwritten or deleted but can be renamed: the old file
stays as ``yavs.old.exe`` while it runs, and the next update deletes it.

If the install directory is not on your ``PATH``, the script says so. In a terminal it asks, for example
``Add $HOME/.local/bin to PATH in ~/.bashrc? [y/N]`` (the startup file of your shell: ``~/.bashrc``, ``~/.zshrc``,
``~/.config/fish/config.fish`` or ``~/.profile``) or, on Windows, ``Add <dir> to your user PATH? [y/N]``. The default is no.
On yes it appends one marked line (once, however often you run it) or adds the directory to your user ``PATH`` (never
the machine's, other entries are kept as they are) and says what it changed; open a new terminal to use it. Without a
terminal, in a CI job or with ``YAVS_NO_MODIFY_PATH=1``, it asks nothing, changes nothing and prints the line to add.

The Linux script needs ``curl`` or ``wget``, ``tar`` and the usual tools of a base system; the options are listed by
``sh -s -- --help`` (``curl -fsSL <url> | sh -s -- --version 0.13.0``). If the newest-release lookup stops at GitHub's
rate limit, the script says so; name the version, or set ``GITHUB_TOKEN``. macOS and other systems have no archive: build
from source (below). The scripts are tested on every change in CI against the real releases, on Linux x86_64 and aarch64
and on Windows (``tools/install/test.sh`` and ``test.ps1``).

Releases
~~~~~~~~

To do it by hand instead, download the archive for your platform from the `releases
<https://github.com/ru551n/yavs/releases>`__ page (releases are marked as pre-releases while yavs is
early), check it against ``SHA256SUMS``, unpack it and put the directory it creates on ``PATH``:

* ``yavs-<version>-x86_64-unknown-linux-gnu.tar.gz``
* ``yavs-<version>-aarch64-unknown-linux-gnu.tar.gz``
* ``yavs-<version>-x86_64-pc-windows-msvc.zip``
* ``yavs-<version>-x86_64-unknown-linux-gnu-compat.tar.gz`` and ``yavs-<version>-x86_64-pc-windows-msvc-compat.zip``:
  the same for x86_64 CPUs without AVX2 (see Requirements)

Pick the x86_64 archive without ``-compat`` unless your CPU is older than 2013 (Intel Haswell, AMD Excavator or Zen); the
``-compat`` one runs on any x86-64 CPU.

aarch64 is also known as arm64; run ``uname -m`` to see which one you need. There is no macOS
archive and no 32-bit build (see From source for macOS).

.. code-block:: bash

    sha256sum -c SHA256SUMS --ignore-missing     # Linux: checks the archives you downloaded
    tar xzf yavs-<version>-x86_64-unknown-linux-gnu.tar.gz
    export PATH="$PWD/yavs-<version>-x86_64-unknown-linux-gnu:$PATH"
    yavs --version

On Windows, compare ``(Get-FileHash yavs-<version>-x86_64-pc-windows-msvc.zip).Hash`` in
PowerShell with the line of ``SHA256SUMS``, unzip the archive and add its directory to ``PATH``.
``yavs --version`` prints the version and the kind of archive on the first line, for example
``yavs 0.20.0 (compiled tier: cranelift)``, the CPU level of the build on the second (``Build: x86-64-v3, AVX2 support``,
``Build: x86-64 baseline`` or ``Build: aarch64``) and the copyright notice on the third.

Every archive has everything in it: nothing else needs to be installed. Its
``yavs`` has the compiled tier built in (:doc:`running-faster`). A release also carries the VS Code
extension of the same version (:doc:`debugging`), and ``pip install vunit-yavs`` gets the VUnit package (:doc:`vunit`).

Every archive is a profile-guided build: it is built, trained on testbenches bundled with the source on its own platform and
CPU level, and built again with what those runs recorded. The speed this gives on each platform is on :doc:`performance`.

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
   * - ``LICENSE-MIMALLOC``
     - Windows archives after 0.19.1 only: the licence (MIT) of the memory allocator built into ``yavs.exe``.
   * - ``README.txt``
     - A few lines on this archive: version, target, CPU variant, the files, how to start.

Requirements
^^^^^^^^^^^^

* The x86_64 archives without ``-compat`` (Linux and Windows) are built for the x86-64-v3 level of the instruction set: the CPU needs
  AVX2, BMI1, BMI2, FMA, F16C, LZCNT and MOVBE, as every Intel CPU since Haswell (2013) and every AMD CPU since
  Excavator (2015) and Zen has. Some low-end Intel Pentium, Celeron and Atom parts of later years lack AVX2, and
  virtual machines or emulators may hide it from the guest. On a CPU without them ``yavs`` prints which features
  are missing and that the ``-compat`` archive or a build from source runs there, then exits with status 1
  (:doc:`troubleshooting`). The ``-compat`` archives are built for the baseline x86-64 CPU and run on any x86-64 CPU.
  The aarch64 archive has no such requirement, and neither has a build from source (below).
* The Linux archives require glibc 2.17 or newer (RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+). On
  musl-based systems such as Alpine, install the ``gcompat`` package. The archives are tested on
  x86_64 and aarch64 Linux and on Windows x86_64.
* ``yavs`` needs nothing else installed: no VHDL libraries, no Python.

Updating and removing
^^^^^^^^^^^^^^^^^^^^^

With the one-line install, run it again (:ref:`one-line-install`). By hand, unpack the new archive in a new directory and change ``PATH``. A library directory written by a yavs
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

The release build optimises the whole program at the end (link-time optimisation), as the archives after 0.19.1 are: it takes a few
minutes and about 6 GB of memory in one compiler process. For a quicker build that runs a little slower, add
``CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`` in front of ``cargo build``.

A build from source targets the baseline x86-64 CPU, so it runs on CPUs older than the archives need. It
can be faster on yours with ``RUSTFLAGS="-C target-cpu=native"``, at the cost of running on that CPU generation
and newer only.

Without ``--features clift`` the build has no compiled tier: ``yavs --version`` says
``interpreter only, no compiled tier``, the default is ``--tier=interp``, and ``--tier=auto`` is
error ``E0120``.

Profile-guided build
^^^^^^^^^^^^^^^^^^^^

A build from source can be profile-guided as well, as the release archives are. ``tools/pgo/build.sh`` builds
yavs with instrumentation, runs a set of bundled testbenches with it, and builds again using what
the runs recorded. It needs the ``llvm-tools`` component of the Rust toolchain, a ``bash`` (on Windows the one of Git for Windows) and Python 3:

.. code-block:: bash

    rustup component add llvm-tools
    tools/pgo/build.sh target/yavs-pgo     # the profile-guided yavs

It takes about three times the processor time of a plain release build (two builds and the training runs) and the
result gives the same output as a plain build. Set ``RUSTFLAGS`` (for example ``-C target-cpu=x86-64-v3``) to give both builds the
same flags. The script fails and says why when a training run fails or when the final build did not use the profile;
it never leaves a binary built without one. The command above was run on Linux x86_64; the release archives run the same script on
Linux x86_64, Linux aarch64 and Windows x86_64 with a build command of their own.

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
      = note: LRM 12.3: without a visible declaration of this identifier, the occurrence is not legal here
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
