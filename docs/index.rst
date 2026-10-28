About yavs
==========

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

yavs ("Yet Another VHDL Simulator") is a VHDL-1993, VHDL-2008 and VHDL-2019 simulator written in
Rust. It is early: a pre-release for people who want to try it and report what breaks.

Key features
------------

* VHDL-1993 (``--std=93``), VHDL-2008 and VHDL-2019, mixed in one project: each file keeps its own
  standard. A subset of PSL is supported; :doc:`language-support` lists what works.
* :doc:`Diagnostics <usage>` with stable codes, source snippets, help, and VHDL call stacks
  for runtime errors. Fail early, fail clearly: nothing runs after a failed step.
* The IEEE packages (``std_logic_1164``, ``numeric_std``, ``math_real``, ``fixed_pkg``,
  ``float_pkg`` and the rest) built into the executable and analyzed from source on demand.
* An interpreter and a compiled tier (LLVM or Cranelift) that compiles the hot code while the
  simulation runs, with output identical on every tier: see :doc:`install` and :doc:`performance`.
* FST waveforms, :doc:`code coverage <coverage>`, a Debug Adapter Protocol server for editors, and
  :doc:`VHPI plugins <vhpi>`.
* Parallel process evaluation (``--threads N``) with output identical to a single-threaded run,
  see :doc:`parallel`.
* One executable for Linux x86_64, Linux aarch64 and Windows x86_64, see :doc:`install`.

Getting started
---------------

Install yavs (:doc:`install`), then analyze the files into a library and run a top level. The
:ref:`quickstart <quickstart>` has the two example files, the output, a failing check and an error.

.. code-block:: bash

    yavs analyze --work=work:build/work hello_tb.vhd
    yavs run --work=work:build/work hello_tb

.. toctree::
   :caption: User guide
   :hidden:

   install
   usage
   recipes
   language-choices

.. toctree::
   :caption: Reference
   :hidden:

   language-support
   conformance
   performance
   parallel
   vhpi
   coverage

.. toctree::
   :caption: About
   :hidden:

   release_notes
   GitHub <https://github.com/ru551n/yavs>
