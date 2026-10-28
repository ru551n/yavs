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
``yavs 0.10.0 (compiled tier: cranelift)``. Next: :doc:`getting-started`.

Every platform has one archive with everything in it: nothing else needs to be installed. Its
``yavs`` has the compiled tier built in (:doc:`running-faster`).

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
     - An early graphical debugger. ``yavs run --gui`` looks for it
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
* ``yavs`` needs nothing else installed: no VHDL libraries, no Python.
* ``yavs-gui`` needs a display (X11 or Wayland on Linux) and an OpenGL driver; software rendering
  works. It loads those libraries when it starts, so it needs no development packages.

Updating and removing
~~~~~~~~~~~~~~~~~~~~~

Unpack the new archive in a new directory and change ``PATH``. A library directory written by a yavs
whose library format differs is refused with error ``L0004`` naming the directory: delete it and
analyze its files again (libraries are not converted). The compilation cache of another yavs build is
ignored, never an error. To remove yavs, delete the directory; ``yavs cache clean --work=NAME:PATH``
removes what it cached in a library and in the shared ``std``/``ieee`` store.

From source
-----------

You need a recent stable Rust toolchain and a C compiler. Build with the Cranelift tier, as the
release archives are:

.. code-block:: bash

    git clone https://github.com/ru551n/yavs
    cd yavs
    cargo build --release -p yavs --features clift   # target/release/yavs
    cd gui && cargo build --release                  # gui/target/release/yavs-gui (optional)

Without ``--features clift`` the build has no compiled tier: ``yavs --version`` says
``interpreter only, no compiled tier``, the default is ``--tier=interp``, and ``--tier=auto`` is
error ``E0120``. The graphical debugger needs network access and ``git`` to fetch one of its
dependencies.

Problems with the installation are covered in :doc:`troubleshooting`.
