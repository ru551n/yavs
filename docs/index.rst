yavs
====

|docs| |repository| |release| |license| |ci|

.. |docs| image:: https://img.shields.io/readthedocs/yavs?label=documentation
   :alt: Documentation
   :target: https://yavs.readthedocs.io
.. |repository| image:: https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github
   :alt: Repository
   :target: https://github.com/ru551n/yavs
.. |release| image:: https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases
   :alt: Release
   :target: https://github.com/ru551n/yavs/releases
.. |license| image:: https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue
   :alt: License
   :target: https://github.com/ru551n/yavs/blob/main/LICENSE-MIT
.. |ci| image:: https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main
   :alt: CI status
   :target: https://github.com/ru551n/yavs/actions/workflows/ci.yml

yavs ("Yet Another VHDL Simulator") is a VHDL simulator written in Rust. It runs VHDL-1993,
VHDL-2008 and VHDL-2019, mixed in one project, from one executable with nothing else to install.
It is early: a pre-release for people who want to try it and report what breaks.

.. code-block:: bash

    yavs analyze --work=work:build/work hello_tb.vhd
    yavs run --work=work:build/work hello_tb

What you get:

* The IEEE packages built in, and the verification libraries OSVVM, UVVM and VUnit running
  unchanged (:doc:`libraries`, :doc:`vunit`).
* Clear diagnostics with stable codes and VHDL call stacks; nothing runs after a failed step
  (:doc:`troubleshooting`).
* :doc:`Static analysis <lint>` of what can be proved wrong before a simulation runs.
* FST :doc:`waveforms`, :doc:`code coverage <coverage>`, a debugger for VS Code and other editors
  (:doc:`debugging`) and :doc:`VHPI plugins <vhpi>`.
* Fast start-up and a compiled tier that compiles the hot code while the simulation runs, with
  identical output on every tier (:doc:`running-faster`, :doc:`threads`).
* Linux x86_64, Linux aarch64 and Windows x86_64.

Where to start:

* New to yavs: :doc:`install`, :doc:`getting-started`, then :doc:`project-walkthrough`.
* Coming from another simulator: :doc:`migrating`.
* Using VUnit: :doc:`vunit`.
* Looking something up: :doc:`cli-reference`, :doc:`error-codes`, :doc:`environment`.

This guide describes the current development version. What is not in a release yet is under "Unreleased" in
the :doc:`release_notes`, and pages say when a feature needs a release newer than 0.11.0.

.. toctree::
   :caption: Get started
   :hidden:

   install
   getting-started
   project-walkthrough
   migrating

.. toctree::
   :caption: Run and debug
   :hidden:

   cli
   generics
   lint
   waveforms
   debugging
   interactive
   coverage
   running-faster
   threads
   troubleshooting

.. toctree::
   :caption: Libraries and integration
   :hidden:

   vunit
   libraries
   mixing-standards
   vhpi
   ci

.. toctree::
   :caption: Reference
   :hidden:

   cli-reference
   lint-rules
   vunit-yavs
   environment
   error-codes
   on-disk
   language-support
   language-choices
   psl

.. toctree::
   :caption: Project
   :hidden:

   conformance
   performance
   release_notes
   faq
   GitHub <https://github.com/ru551n/yavs>
