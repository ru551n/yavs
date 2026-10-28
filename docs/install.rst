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
``yavs 0.8.0 (compiled tier: cranelift)``. The first simulation, from an empty directory, is in
:doc:`usage`.

Which archive
~~~~~~~~~~~~~

Every platform has one archive, and it contains everything: nothing else needs to be installed.
``--tier=auto`` (the default) compiles the hot code to machine code during the run, and
``--tier=clift`` compiles all of it before time 0; ``--tier=interp`` runs the interpreter alone. The
output is the same with every tier. The compiled tier of the archive is Cranelift, inside the
executable. The next section says what the tiers are.

Build variants and tiers
~~~~~~~~~~~~~~~~~~~~~~~~

yavs runs a design in the interpreter and can compile the hot parts to machine code. Whether a
compiled tier exists depends on how your ``yavs`` was built. ``yavs --version`` says which you have
on its first line:

.. code-block:: text


    yavs 0.8.0 (compiled tier: cranelift)
    yavs 0.8.0 (interpreter only, no compiled tier)

.. list-table::
   :header-rows: 1

   * - Build
     - Compiled tier
     - Default ``--tier``
   * - Release archive (``yavs-<version>-<target>``)
     - Cranelift, inside the executable
     - ``auto`` (runs as ``clift-auto``)
   * - ``cargo build --release -p yavs`` (plain source build)
     - none
     - ``interp``
   * - Source build with ``--features clift``
     - Cranelift
     - ``auto``

``--tier=auto`` starts the run in the interpreter. A sampler watches where the interpreter spends its
time, and one background thread compiles those functions and swaps them in while the simulation
keeps running; a run shorter than about 5 ms creates no compile thread. Elaboration and package
initialisation are always interpreted. The output is the same in every tier. Compiled code is kept
in the work library's cache (``yavs-cache/obj/``), so a repeated run links it instead of compiling it
again (``--no-cache`` turns that off).

``--tier=clift`` compiles everything before time 0 instead. ``--tier=interp`` never compiles. Asking
for a tier the build does not have is error ``E0120``; the plain source build therefore rejects
``--tier=auto`` and ``--tier=clift``, but runs without any ``--tier`` option. A debugger session
(``--dap``) always runs the interpreter: the default is ``interp`` and an explicit ``--tier`` has no
effect there (a known limitation).

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
   * - ``LICENSE-MIT``, ``LICENSE-APACHE``
     - The licences of yavs, MIT or Apache-2.0 at your option.
   * - ``LICENSE-IEEE``, ``LICENSE-IEEE-VITAL``, ``LICENSE-SYNOPSYS``
     - The licence and notices of the built-in ``std`` and ``ieee`` sources, the VITAL packages and the
       Synopsys packages.
   * - ``LICENSE-EUPL-1.2-yavs-gui``
     - The licence of ``yavs-gui``, EUPL-1.2. It applies to the ``yavs-gui`` binary only; everything
       else is MIT or Apache-2.0.
   * - ``README.md``
     - The project readme.

Requirements
~~~~~~~~~~~~

* The Linux archives require glibc 2.17 or newer (RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+). On
  musl-based systems such as Alpine, install the ``gcompat`` package. The archives are tested on
  x86_64 and aarch64 Linux and on Windows x86_64.
* ``yavs`` needs nothing else installed: no VHDL libraries, no Python. Python
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
default tier is ``--tier=interp``, and ``--tier=auto`` and ``--tier=clift`` stop with error E0120. The
graphical debugger needs Rust 1.95 or newer (``rust-version`` in ``gui/Cargo.toml``) and network access
and ``git``: one of its dependencies is fetched from a git repository.

With the Cranelift tier
~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: bash

    cargo build --release -p yavs --features clift

This is how the release archives are built. It needs nothing but Rust: ``yavs --version`` says
``compiled tier: cranelift`` and ``--tier=auto`` (the default) compiles the hot code with Cranelift.

If something goes wrong
-----------------------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - What you see
     - What to do
   * - ``yavs: command not found``
     - The directory of the archive is not on ``PATH``.
   * - ``error[E0120]``
     - A build from source without the ``clift`` feature has no compiled tier. Build with
       ``--features clift`` or use ``--tier=interp``.
   * - ``error[E0122]: `yavs-gui` was not found``
     - ``yavs-gui`` is not next to ``yavs`` and not on ``PATH``.
   * - ``error[L0001]: library `work` not found``
     - Nothing was analyzed into that ``--work`` directory yet, or the path differs from the one given to
       ``analyze``.
   * - ``error[L0004]``
     - The library was written by another yavs library format: delete the directory and analyze again.
   * - Alpine: ``yavs: not found`` although the file exists
     - Install ``gcompat``.
