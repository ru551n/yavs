Continuous integration
======================

This page has recipes for running yavs in CI: what the exit status means, a coverage gate, what to
cache between runs and a complete GitHub Actions job built from plain steps.

.. contents:: On this page
   :local:

Exit status
-----------

Every ``yavs`` command says how it went with its exit status, so a CI step fails by itself:

.. list-table::
   :header-rows: 1
   :widths: 15 85

   * - Status
     - Meaning
   * - 0
     - Success.
   * - 1
     - A file has errors, or a failed assertion, a report at the exit severity or a runtime error, or a
       total coverage below ``--fail-under``.
   * - 2
     - A usage, library or elaboration error.
   * - 3
     - An internal error of yavs: please report it.
   * - N
     - ``stop(N)`` or ``finish(N)`` in the design, N from 1 to 125.
   * - 130
     - Stopped by Ctrl-C or SIGTERM.
   * - 141
     - Standard output was closed.

:doc:`cli-reference` has the exact wording. A pipe hides the status of ``yavs``: in a shell step use
``set -o pipefail``, as GitHub Actions does for ``bash`` when you write ``shell: bash``. ``-Werror`` makes
warnings fail the step too.

Getting yavs
------------

Pin the version. Releases are pre-releases for now, so there is no "latest" to follow, and a pinned version
also keeps the cache valid between runs:

.. code-block:: bash

    V=0.11.0
    A=yavs-$V-x86_64-unknown-linux-gnu
    curl -fsSLO https://github.com/ru551n/yavs/releases/download/v$V/$A.tar.gz
    curl -fsSLO https://github.com/ru551n/yavs/releases/download/v$V/SHA256SUMS
    sha256sum -c SHA256SUMS --ignore-missing
    tar xzf $A.tar.gz
    export PATH="$PWD/$A:$PATH"
    yavs --version

:doc:`install` lists the archives of the other platforms.

A coverage gate
---------------

.. note::

   ``--fail-under``, ``--total`` and ``--kind`` of ``yavs cov report``, and the coverage flags of
   ``vunit-yavs``, need a release newer than 0.11.0. On 0.11.0 the report prints but does not gate.

.. code-block:: bash

    yavs run --work=work:build/work --map=rtl:build/rtl counter_tb --cov=cov/a.ycov
    yavs cov merge cov/*.ycov -o cov/all.ycov
    yavs cov report cov/all.ycov --html cov-html --cobertura coverage.xml --fail-under=85
    echo "coverage $(yavs cov report cov/all.ycov --total)"

Below the limit the report is printed as usual and the exit status is 1. Percentages are rounded down, so
a gate never passes by rounding up. With VUnit the same gate is one command line
(:doc:`vunit`):

.. code-block:: bash

    python run.py --yavs-cov --yavs-cov-html cov-html --yavs-cov-fail-under 85

What to cache
-------------

Two things can be kept between CI runs, and both are safe to lose (:doc:`on-disk`):

* The library directories, for example ``build/``. They hold the compile cache, ``yavs-cache/``.
* The shared store of ``std`` and ``ieee``. Set ``YAVS_STD_CACHE`` to a directory you choose, so that it is
  easy to name in the cache step.

Then run all your ``analyze`` commands in every job. yavs compares each file by its hash, so a file that did
not change costs almost nothing, and one that did is analyzed again. A fresh checkout with new modification
times changes nothing.

A library records absolute paths. A cache restored at another path, such as another runner's checkout
directory, makes ``analyze`` read the files again. That is slower, not wrong.

Key the cache on the yavs version and on your sources, and let an older entry serve as a start. An
entry of another yavs is ignored, or refused with ``L0004`` if its library format differs. In that case delete the
directory and analyze again.

A GitHub Actions job
--------------------

This is the walkthrough project (:doc:`project-walkthrough`) as a job. It uses plain steps and needs nothing
but ``actions/checkout``, ``actions/cache`` and ``actions/upload-artifact``.

.. code-block:: yaml

    name: simulate
    on: [push, pull_request]

    jobs:
      simulate:
        runs-on: ubuntu-latest
        env:
          YAVS_VERSION: 0.11.0
          YAVS_STD_CACHE: ${{ github.workspace }}/.yavs-std
        steps:
          - uses: actions/checkout@v4

          - name: Install yavs
            run: |
              A=yavs-$YAVS_VERSION-x86_64-unknown-linux-gnu
              curl -fsSLO https://github.com/ru551n/yavs/releases/download/v$YAVS_VERSION/$A.tar.gz
              curl -fsSLO https://github.com/ru551n/yavs/releases/download/v$YAVS_VERSION/SHA256SUMS
              sha256sum -c SHA256SUMS --ignore-missing
              tar xzf $A.tar.gz
              echo "$PWD/$A" >> "$GITHUB_PATH"

          - uses: actions/cache@v4
            with:
              path: |
                build
                .yavs-std
              key: yavs-${{ env.YAVS_VERSION }}-${{ hashFiles('rtl/**', 'tb/**') }}
              restore-keys: yavs-${{ env.YAVS_VERSION }}-

          - name: Analyze
            run: |
              yavs analyze --work=rtl:build/rtl rtl/counter_pkg.vhd rtl/counter.vhd
              yavs analyze --work=work:build/work --map=rtl:build/rtl tb/counter_tb.vhd

          - name: Simulate
            run: |
              mkdir -p cov
              yavs run --work=work:build/work --map=rtl:build/rtl counter_tb -gWIDTH=3 --cov=cov/w3.ycov
              yavs run --work=work:build/work --map=rtl:build/rtl counter_tb -gWIDTH=4 --cov=cov/w4.ycov

          - name: Coverage
            run: |
              yavs cov merge cov/*.ycov -o cov/all.ycov
              yavs cov report cov/all.ycov --html cov-html
              # With a release newer than 0.11.0, add --fail-under=85 to make it a gate

          - uses: actions/upload-artifact@v4
            if: always()
            with:
              name: coverage
              path: cov-html

The same job with VUnit replaces the three yavs steps with ``pip install vunit-yavs`` and
``VUNIT_SIMULATOR=yavs python run.py -p 4 --xunit-xml results.xml``.

See also
--------

* :doc:`on-disk`: what is in the directories you cache.
* :doc:`coverage`: exclusions and report formats.
* :doc:`vunit`: running tests with VUnit.
* :doc:`cli-reference`: the exit statuses.
