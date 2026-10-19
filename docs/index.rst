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

yavs ("Yet Another VHDL Simulator") is a VHDL-2008 and VHDL-2019 simulator written in Rust. It
is early: a pre-release for people who want to try it and report what breaks.

Key features
------------

* VHDL-2008 and VHDL-2019, mixed in one project: each file is parsed with its own standard.
* :doc:`Diagnostics <usage>` with stable codes, source snippets, help, and VHDL call stacks
  for runtime errors. Fail early, fail clearly: nothing runs after a failed step.
* The IEEE packages (``std_logic_1164``, ``numeric_std``, ``math_real``, ``fixed_pkg``,
  ``float_pkg`` and the rest) built into the executable and analyzed from source on demand.
* Parallel process evaluation (``--threads N``) with output identical to a single-threaded run.
* One executable for Linux x86_64, Linux aarch64 and Windows x86_64, see :doc:`install`.

Getting started
---------------

.. code-block:: bash

    yavs analyze --std=2008 --work=work:out/work tb.vhd
    yavs run --work=work:out/work tb

.. toctree::
   :caption: User guide
   :hidden:

   install
   usage
   recipes
   language-choices

.. toctree::
   :caption: About
   :hidden:

   release_notes
   GitHub <https://github.com/ru551n/yavs>
