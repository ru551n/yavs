Installation
============

Releases
--------

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
``yavs --version`` prints the version and the kind of archive, for example
``yavs 0.8.0 (compiled tier: llvm)``. The first simulation, from an empty directory, is in
:doc:`usage`.

Which archive
~~~~~~~~~~~~~

Every platform has one archive, and it contains everything: nothing else needs to be installed.
``--tier=auto`` (the default) compiles the hot code to machine code during the run, and
``--tier=clift`` compiles all of it before time 0; ``--tier=interp`` runs the interpreter alone. The
output is the same with every tier. ``yavs --version`` shows the tier on its first line::

    yavs 0.8.0 (compiled tier: cranelift)

A yavs you build yourself without either feature prints ``yavs 0.8.0 (interpreter only, no compiled
tier)``; see `From source`_.

What is in the archive
~~~~~~~~~~~~~~~~~~~~~~

Each archive unpacks into one directory of the same name (without ``.tar.gz`` or ``.zip``):

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - File
     - What it is
   * - ``yavs``, ``yavs.exe``
     - The simulator. The ``std`` and ``ieee`` sources are built into it.
   * - ``yavs-gui``, ``yavs-gui.exe``
     - The graphical debugger (:doc:`usage`, "Graphical debugger"). ``yavs run --gui`` looks for it
       next to ``yavs``, so keep the two together.
   * - ``lib/libLLVM.so.21.1``
     - The LLVM library, in the Linux archive with the compiled tier only.
   * - ``LICENSE-MIT``, ``LICENSE-APACHE``
     - The licences of yavs, MIT or Apache-2.0 at your option.
   * - ``LICENSE-IEEE``, ``LICENSE-IEEE-VITAL``, ``LICENSE-SYNOPSYS``
     - The licence and notices of the built-in ``std`` and ``ieee`` sources, the VITAL packages and the
       Synopsys packages.
   * - ``LICENSE-EUPL-1.2-yavs-gui``
     - The licence of ``yavs-gui``, EUPL-1.2. It applies to the ``yavs-gui`` binary only; everything
       else is MIT or Apache-2.0.
   * - ``LICENSE-LLVM``
     - Apache-2.0 with LLVM exceptions, in the archives with the compiled tier only.
   * - ``README.md``
     - The project readme.

Requirements
~~~~~~~~~~~~

* The Linux archives require glibc 2.17 or newer (RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+). On
  musl-based systems such as Alpine, install the ``gcompat`` package. The archives are tested on
  x86_64 and aarch64 Linux and on Windows x86_64.
* ``yavs`` needs nothing else installed: no VHDL libraries, no LLVM, no Python. Python
  testbench frameworks and VHPI plugins are separate (:doc:`recipes`, :doc:`usage`).
* ``yavs-gui`` needs a display (X11 or Wayland on Linux) and an OpenGL driver; software rendering
  works. It loads those libraries when it starts, so it needs no development packages.

Environment
~~~~~~~~~~~

yavs reads these environment variables; none is needed for a normal install.

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Variable
     - Meaning
   * - ``YAVS_STDLIB``
     - Use the ``std`` and ``ieee`` sources in this directory instead of the built-in copy, laid out as
       ``<dir>/{std,ieee}/{1993,2008,2019}/*.vhdl`` (the ``lib`` directory of the source tree has that
       layout). The compilation cache is off while it is set.
   * - ``YAVS_STD_CACHE``
     - Where the analyzed ``std`` and ``ieee`` units are kept once per user: ``yavs/std`` in
       ``$XDG_CACHE_HOME`` or ``~/.cache`` (``%LOCALAPPDATA%`` on Windows) when unset. ``off`` keeps
       them in each library; ``--std-cache=DIR|off`` wins. An empty value or an unwritable directory is
       error ``E0126``.
   * - ``NO_COLOR``
     - When set and not empty, diagnostics have no colour (``--color=always`` still forces it).
   * - ``YAVS_WAVE_FLUSH``
     - ``5s`` or ``500ms`` also ends a waveform block after that much run time, so a killed run leaves
       more of the file; ``never`` (default) ends blocks by size only. Any other value is error
       ``E0801``.
   * - ``YAVS_HUGE_HEAP``
     - ``0`` stops yavs from asking for transparent huge pages on its heap (Linux only).

Updating and removing
~~~~~~~~~~~~~~~~~~~~~

Unpack the new archive in a new directory and change ``PATH``. A library directory written by a yavs
whose library format differs is refused with error ``L0004`` naming the directory: delete it and
analyze its files again (libraries are not converted). The compilation cache of another yavs build is
ignored, never an error. To remove yavs, delete the directory; ``yavs cache clean --work=NAME:PATH``
removes what it cached in a library and in the shared ``std``/``ieee`` store.

From source
-----------

You need a recent stable Rust toolchain (the workspace uses edition 2024; the CI builds with the
current stable) and a C compiler (a small C file of the VHPI layer is compiled at build time).

.. code-block:: bash

    git clone https://github.com/ru551n/yavs
    cd yavs
    cargo build --release -p yavs   # target/release/yavs
    cd gui && cargo build --release  # gui/target/release/yavs-gui (optional)

This yavs has no compiled tier: ``yavs --version`` says ``interpreter only, no compiled tier``, the
default tier is ``--tier=interp``, and ``--tier=auto`` and ``--tier=llvm`` stop with error E0120. The
graphical debugger needs Rust 1.95 or newer (``rust-version`` in ``gui/Cargo.toml``) and network access
and ``git``: one of its dependencies is fetched from a git repository.

With the Cranelift tier
~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: bash

    cargo build --release -p yavs --features clift

This is how the ``-nollvm`` archives are built. It needs nothing but Rust: ``yavs --version`` says
``compiled tier: cranelift`` and ``--tier=auto`` (the default) compiles the hot code with Cranelift.

With the LLVM tier
~~~~~~~~~~~~~~~~~~

``yavs run --tier=llvm`` needs a build with the ``llvm`` feature and LLVM 21.1 (pinned). Point
``LLVM_SYS_211_PREFIX`` at the install if ``llvm-config`` is not on ``PATH``.

.. code-block:: bash

    # Linux x86_64 and aarch64 (apt.llvm.org packages, as the CI installs them)
    sudo apt-get install llvm-21-dev libzstd-dev
    export LLVM_SYS_211_PREFIX=/usr/lib/llvm-21
    cargo build --release -p yavs --features llvm

On Linux the build needs the shared LLVM library (``llvm-config --link-shared``). The executable does
not link it: it opens ``libLLVM.so.21.1`` the first time the compiled tier is used, from ``lib/`` next to
the executable, then through the dynamic loader's search, then at the path of the build, and if none
loads, ``--tier=llvm`` stops with error ``E0127`` and ``--tier=auto`` warns once and keeps the hot code
interpreted. Both features can be built together (``--features llvm,clift``), which adds
``--tier=auto3``.

On Windows (x86_64-pc-windows-msvc) use the official ``clang+llvm-21.1.8-x86_64-pc-windows-msvc``
release archive (static libraries, ``llvm-config.exe``) with ``RUSTFLAGS="-C
target-feature=+crt-static"``. The archive lacks ``libxml2s.lib``: create an empty one with
``llvm-lib /llvmlibempty /out:<prefix>\lib\libxml2s.lib``, and copy ``psapi``, ``shell32``,
``ole32``, ``uuid``, ``advapi32``, ``ws2_32`` and ``ntdll`` ``.lib`` from the Windows SDK into
``<prefix>\lib`` (see the ``llvm`` job in ``.github/workflows/ci.yml`` for the exact steps).

If something goes wrong
-----------------------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - What you see
     - What to do
   * - ``yavs: command not found``
     - The directory of the archive is not on ``PATH``.
   * - ``error[E0120]: `--tier=llvm` needs a yavs built with the LLVM tier``
     - You run a ``-nollvm`` archive or a build without the feature. Use ``--tier=auto`` or
       ``--tier=clift``, or the archive without ``-nollvm``.
   * - ``error[E0127]: the LLVM library cannot be loaded``
     - The ``lib`` directory was separated from ``yavs``. Put it back next to it, or use
       ``--tier=interp``.
   * - ``error[E0122]: `yavs-gui` was not found``
     - ``yavs-gui`` is not next to ``yavs`` and not on ``PATH``.
   * - ``error[L0001]: library `work` not found``
     - Nothing was analyzed into that ``--work`` directory yet, or the path differs from the one given to
       ``analyze``.
   * - ``error[L0004]``
     - The library was written by another yavs library format: delete the directory and analyze again.
   * - Alpine: ``yavs: not found`` although the file exists
     - Install ``gcompat``.
