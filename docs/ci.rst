Continuous integration
======================

This page has recipes for running yavs in CI: what the exit status means, a coverage gate, what to
cache between runs and a complete GitHub Actions job.

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
     - Stopped by Ctrl-C or SIGTERM (with the compiled tier, also from a plain terminal: yavs ends the run itself after storing the compilation cache's lists; a run with ``--wave`` ends with status 1 instead and finishes its file, yavs issue #1155).
   * - 141
     - Standard output was closed.

:doc:`cli-reference` has the exact wording. A pipe hides the status of ``yavs``: in a shell step use
``set -o pipefail``, as GitHub Actions does for ``bash`` when you write ``shell: bash``. ``-Werror`` makes
warnings fail the step too.

.. _ci-getting-yavs:

Getting yavs
------------

On GitHub Actions, use `yavs-action <https://github.com/ru551n/yavs-action>`__. It installs a release on
Linux x86_64, Linux aarch64 or Windows x86_64, verifies it against ``SHA256SUMS``, checks that it is the
version you asked for and puts it on ``PATH``:

.. code-block:: yaml

    - uses: ru551n/yavs-action@v1
      with:
        version: 0.19.1

Pin the version: it gives the same simulator on every run and keeps your cache valid. Without ``version`` you
get the newest stable release, which can change your results on the day a release comes out. An unknown
version, a runner without an archive or a failed download fails the step with an error naming the cause and
the fix. The action's page has the outputs and the rest. The x86_64 archives need a CPU with AVX2
(x86-64-v3, :ref:`installation`): GitHub-hosted runners have it. On a self-hosted runner without AVX2, set
``variant: compat`` to install the ``-compat`` archive, which runs on any x86-64 CPU, or build yavs from source:

.. code-block:: yaml

    - uses: ru551n/yavs-action@v1
      with:
        version: 0.19.1
        variant: compat

``variant`` is ``default`` or ``compat``; any other value fails the step. ``compat`` needs yavs 0.14.0 or
newer and fails for an older version. On Linux aarch64 there is one archive, so ``compat`` installs it and
prints a notice.

On other CI systems, download the archive with plain commands:

.. code-block:: bash

    V=0.19.1
    A=yavs-$V-x86_64-unknown-linux-gnu
    curl -fsSLO https://github.com/ru551n/yavs/releases/download/v$V/$A.tar.gz
    curl -fsSLO https://github.com/ru551n/yavs/releases/download/v$V/SHA256SUMS
    sha256sum -c SHA256SUMS --ignore-missing
    tar xzf $A.tar.gz
    export PATH="$PWD/$A:$PATH"
    yavs --version

For a CPU without AVX2 use ``A=yavs-$V-x86_64-unknown-linux-gnu-compat``, as ``variant: compat`` does in the action.

The one-line installer (:ref:`one-line-install`) does the same on Linux x86_64 and aarch64 with the checksum check, and
picks the ``-compat`` archive by itself on a CPU without AVX2. It asks no question without a terminal, so it can run in a
job; pin the version and put the directory on ``PATH`` yourself:

.. code-block:: bash

    curl -fsSL https://raw.githubusercontent.com/ru551n/yavs/main/install.sh | sh -s -- --version 0.14.0
    export PATH="$HOME/.local/bin:$PATH"
    yavs --version

Looking up the newest version uses GitHub's API, which limits requests without a token; ``--version`` needs no lookup.

:ref:`installation` lists the archives of the other platforms.

A coverage gate
---------------

.. note::

   ``--fail-under``, ``--total`` and ``--kind`` of ``yavs cov report``, and the coverage flags of
   ``vunit-yavs``, need 0.12.0 or later. On 0.11.0 the report prints but does not gate.

.. code-block:: bash

    yavs run --work=work:build/work --map=rtl:build/rtl counter_tb --cov=sbt --cov-file=cov/a.ycov
    yavs cov merge cov/*.ycov -o cov/all.ycov
    yavs cov report cov/all.ycov --html cov-html --cobertura coverage.xml --fail-under=85
    echo "coverage $(yavs cov report cov/all.ycov --total)"

Below the limit the report is printed as usual and the exit status is 1. Percentages are rounded down, so
a gate never passes by rounding up. ``--fail-under=statement:90,branch:80`` gates each kind on its own and
names the ones that fall short. ``--json``, ``--lcov`` and ``--sonarqube`` write other exports for your CI service
(:doc:`coverage`; the LCOV export was checked with lcov 1.16, the SonarQube export has not been read by a real scanner). With VUnit the same gate is one command line
(:doc:`vunit`):

.. code-block:: bash

    python run.py --yavs-cov=sbt --yavs-cov-html cov-html --yavs-cov-fail-under 85

A lint step
-----------

``yavs lint`` (0.12.0 or later) fails a step with a finding of severity error, and writes reports a pipeline can
read (:doc:`lint`):

.. code-block:: bash

    yavs lint --work=rtl:build/rtl --sarif=lint.sarif

What to cache
-------------

Two things can be kept between CI runs, and both are safe to lose (:doc:`on-disk`):

* The library directories, for example ``build/``. They hold the compile cache, ``yavs-cache/``.
  A large suite can make that more than a GB per library directory, up to the limit in :doc:`on-disk`.
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

This is the walkthrough project (:doc:`project-walkthrough`) as a job. It needs nothing but ``yavs-action``,
``actions/checkout``, ``actions/cache`` and ``actions/upload-artifact``.

.. code-block:: yaml

    name: simulate
    on: [push, pull_request]

    jobs:
      simulate:
        runs-on: ubuntu-latest
        env:
          YAVS_VERSION: 0.19.1
          YAVS_STD_CACHE: ${{ github.workspace }}/.yavs-std
        steps:
          - uses: actions/checkout@v4

          - uses: ru551n/yavs-action@v1
            with:
              version: ${{ env.YAVS_VERSION }}

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
              yavs run --work=work:build/work --map=rtl:build/rtl counter_tb -gWIDTH=3 --cov=sbt --cov-file=cov/w3.ycov
              yavs run --work=work:build/work --map=rtl:build/rtl counter_tb -gWIDTH=4 --cov=sbt --cov-file=cov/w4.ycov

          - name: Coverage
            run: |
              yavs cov merge cov/*.ycov -o cov/all.ycov
              yavs cov report cov/all.ycov --html cov-html --fail-under=85

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
