Installation
============

Releases
--------

Download the archive for your platform from the `releases
<https://github.com/ru551n/yavs/releases>`__ page, check it against ``SHA256SUMS``, unpack it and
put the directory on ``PATH``:

* ``yavs-<version>-x86_64-unknown-linux-gnu.tar.gz``
* ``yavs-<version>-aarch64-unknown-linux-gnu.tar.gz``
* ``yavs-<version>-x86_64-pc-windows-msvc.zip``

aarch64 is also known as arm64; run ``uname -m`` to see which one you need.

Which archive
~~~~~~~~~~~~~

Every platform has two archives. The one named as above has the compiled tier: ``--tier=auto``
(the default) compiles the hot code with LLVM 21 during the run and ``--tier=llvm`` compiles all of it
before time 0. The Linux archive carries LLVM as ``lib/libLLVM.so.21.1`` next to the
executable (keep the ``lib`` directory with ``yavs``; it needs nothing else installed), the Windows
executable has LLVM linked in. The archive with ``-nollvm`` added before the extension
(``yavs-<version>-<target>-nollvm.tar.gz``, ``.zip`` on Windows) has no LLVM inside and is smaller: its
compiled tier is Cranelift, and ``--tier=auto`` (the default there too) compiles the hot code with it
during the run, ``--tier=clift`` all of it before time 0. Its compiled code is mostly as fast as LLVM's;
tight compute loops run faster with LLVM. ``--tier=llvm`` stops with error E0120 in it.
``yavs --version`` shows the difference on its first line::

    yavs 0.8.0 (compiled tier: llvm)
    yavs 0.8.0 (compiled tier: cranelift)

The Linux archives require glibc 2.17 or newer (RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+).
On musl-based systems such as Alpine, install the ``gcompat`` package.

.. code-block:: bash

    sha256sum -c SHA256SUMS --ignore-missing

The archive holds the ``yavs`` executable (with ``lib/`` and ``LICENSE-LLVM`` in the archive with the compiled tier), the graphical debugger ``yavs-gui`` and the
licences. ``yavs-gui`` is licensed under the EUPL-1.2 (``LICENSE-EUPL-1.2-yavs-gui``, which
applies to the ``yavs-gui`` binary only); everything else is MIT or Apache-2.0. The ``std`` and
``ieee`` sources are built into the executable; nothing else needs to be installed. ``YAVS_STDLIB`` points yavs at
another copy of the tree (``<dir>/{std,ieee}/{1993,2008,2019}/*.vhdl``).

From source
-----------

With Rust stable and a C compiler:

.. code-block:: bash

    git clone https://github.com/ru551n/yavs
    cd yavs
    cargo build --release -p yavs   # target/release/yavs
    cd gui && cargo build --release  # gui/target/release/yavs-gui (optional)

With the LLVM tier
~~~~~~~~~~~~~~~~~~

``yavs run --tier=llvm`` needs a build with the ``llvm`` feature and LLVM 21.1 (pinned). Point
``LLVM_SYS_211_PREFIX`` at the install if ``llvm-config`` is not on ``PATH``.

.. code-block:: bash

    # Linux x86_64 and aarch64 (apt.llvm.org package llvm-21-dev, which includes the static libs)
    export LLVM_SYS_211_PREFIX=/usr/lib/llvm-21
    cargo build --release -p yavs --features llvm

On Windows (x86_64-pc-windows-msvc) use the official ``clang+llvm-21.1.8-x86_64-pc-windows-msvc``
release archive (static libraries, ``llvm-config.exe``) with ``RUSTFLAGS="-C
target-feature=+crt-static"``. The archive lacks ``libxml2s.lib``: create an empty one with
``llvm-lib /llvmlibempty /out:<prefix>\lib\libxml2s.lib``, and copy ``psapi``, ``shell32``,
``ole32``, ``uuid``, ``advapi32``, ``ws2_32`` and ``ntdll`` ``.lib`` from the Windows SDK into
``<prefix>\lib`` (see the ``llvm`` job in ``.github/workflows/ci.yml`` for the exact steps).
