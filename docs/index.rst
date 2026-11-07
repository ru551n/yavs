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

* A debugger at a terminal prompt (:doc:`interactive`) and in VS Code and other editors (:doc:`debugging`):
  breakpoints on lines, values and failed assertions, stepping, ``examine``, ``force``, ``drivers``, ``trace x``, and
  ``restart`` to time 0 without recompiling. :doc:`debugger-tutorial` debugs a failing test step by step, and
  :doc:`console-reference` has every command.
* The waveform in Surfer while the run writes it, with the cursor following the debugger (:doc:`waveforms`), or an FST file
  for any viewer.
* :doc:`Static analysis <lint>` of what can be proved wrong before a simulation runs.
* :doc:`Code coverage <coverage>`, up to condition combinations, with merged databases, HTML and Cobertura reports and a
  CI gate.
* Reports at the end of a run: what failed, what keeps it busy, what each process waits for and where the time goes
  (:doc:`cli`).
* The IEEE packages built in, and the verification libraries OSVVM, UVVM and VUnit running
  unchanged (:doc:`libraries`). The ``vunit-yavs`` package adds debugging, coverage and sharing CPU cores to a VUnit run script (:doc:`vunit`).
* Clear diagnostics with stable codes, VHDL call stacks and a "did you mean" for a misspelled name; nothing runs after a failed
  step (:doc:`troubleshooting`).
* :doc:`VHPI plugins <vhpi>` and calls to C functions from VHDL.
* Fast start-up and a compiled tier that compiles the hot code while the simulation runs, with
  identical output on every tier (:doc:`running-faster`, :doc:`threads`).
* Linux x86_64, Linux aarch64 and Windows x86_64.

Where to start:

* New to yavs: :doc:`getting-started`, then :doc:`project-walkthrough`.
* Coming from another simulator: :doc:`migrating`.
* Using VUnit: :doc:`vunit`.
* A failing test to debug: :doc:`debugger-tutorial`.
* Looking something up: :doc:`cli-reference`, :doc:`console-reference`, :doc:`error-codes`, :doc:`environment`.

This guide describes the current development version. What is not in a release yet is under "Unreleased" in
the :doc:`release_notes`.

.. toctree::
   :caption: Get started
   :hidden:

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
   debugger-tutorial
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
   verilog
   vhpi
   ci

.. toctree::
   :caption: Reference
   :hidden:

   cli-reference
   console-reference
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
   Contributing <https://github.com/ru551n/yavs/blob/main/CONTRIBUTING.md>
   GitHub <https://github.com/ru551n/yavs>
