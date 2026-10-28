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
  unchanged (:doc:`libraries`).
* Clear diagnostics with stable codes and VHDL call stacks; nothing runs after a failed step
  (:doc:`troubleshooting`).
* FST :doc:`waveforms`, :doc:`code coverage <coverage>`, a debugger for VS Code and other editors
  (:doc:`debugging`) and :doc:`VHPI plugins <vhpi>`.
* Fast start-up and a compiled tier that compiles the hot code while the simulation runs, with
  identical output on every tier (:doc:`running-faster`).
* Linux x86_64, Linux aarch64 and Windows x86_64.

Start with :doc:`install` and :doc:`getting-started`.

.. toctree::
   :caption: User guide
   :hidden:

   install
   getting-started
   libraries
   waveforms
   coverage
   debugging
   vhpi
   running-faster
   cli
   troubleshooting

.. toctree::
   :caption: Reference
   :hidden:

   language-support
   language-choices
   conformance
   performance

.. toctree::
   :caption: About
   :hidden:

   release_notes
   GitHub <https://github.com/ru551n/yavs>
