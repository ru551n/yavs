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

.. code-block:: bash

    sha256sum -c SHA256SUMS --ignore-missing

The archive holds the ``yavs`` executable and the licences. The ``std`` and ``ieee`` sources are
built into the executable; nothing else needs to be installed. ``YAVS_STDLIB`` points yavs at
another copy of the tree (``<dir>/{std,ieee}/{2008,2019}/*.vhdl``).

From source
-----------

With Rust stable and a C compiler:

.. code-block:: bash

    git clone https://github.com/ru551n/yavs
    cd yavs
    cargo build --release -p yavs   # target/release/yavs
