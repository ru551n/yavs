FAQ and known limitations
=========================

This page answers common questions and lists what yavs does not do. For what a message means, see
:doc:`troubleshooting` and :doc:`error-codes`.

.. contents:: On this page
   :local:

Questions
---------

What does it cost, and do I need a licence server?
    yavs is free software under MIT or Apache-2.0, at your choice. It needs no licence server and no
    account.

What do I have to install?
    One executable. The ``std`` and ``ieee`` libraries are built in. There is nothing else to set up, unless you
    want VUnit (:doc:`vunit`) or the VS Code debugger (:doc:`debugging`).

Which platforms does it run on?
    Linux x86_64, Linux aarch64 and Windows x86_64. There is no macOS build and no 32-bit build.

Which VHDL standards?
    VHDL-1993, VHDL-2008 (the default) and VHDL-2019, mixed in one project (:doc:`mixing-standards`). VHDL-2002 code
    goes under ``--std=2008``, and VHDL-1987 code under ``--std=93``.

Is it finished?
    No. It is a pre-release for people who want to try it and report what breaks. :doc:`conformance` shows what is
    checked, and :doc:`language-support` what works.

How fast is it?
    :doc:`performance` has the measurements and how they were made. Time your own design as well.

Why do reports show times in femtoseconds?
    The default resolution is 1 fs, and reports always use it. ``32000000fs+0`` is 32 ns in delta cycle 0.

Why does my simulation never end?
    A free-running clock always has one more event. Add ``--stop-time``, or call ``std.env.finish`` in the testbench.
    ``--hang-report`` says what the processes wait for when a run stops without finishing.

Can I run several simulations on one library at the same time?
    Yes. ``run`` does not change a library's units, and parallel ``analyze`` calls into one library are safe too.

Is the output the same at every tier and thread count?
    Yes. Output, reports, waves, coverage and the exit status do not depend on ``--tier`` or ``--threads``.

Which waveform format is there?
    FST, in every case, whatever the file is named. There is no VCD output.

Can I use yavs in a script?
    Yes. Each command is an executable call with an exit status (:doc:`cli`, :doc:`ci`).

Where do I report a bug?
    In the `issue tracker <https://github.com/ru551n/yavs/issues>`__, with the smallest input that shows it. An exit
    status of 3 is always a bug in yavs.

Known limitations
-----------------

Languages
~~~~~~~~~

* VHDL only: no Verilog and no SystemVerilog, so no mixed-language designs. Plain Verilog is
  `planned <https://github.com/ru551n/yavs/issues/776>`__.
* VHDL-2019 is mostly there, and VHDL-2008 is the best checked. :ref:`known-gaps` lists what stops with an error.
* PSL is a subset: :doc:`psl`.
* Some invalid code is accepted instead of rejected. Do not use yavs as your only check that code is legal.

Libraries
~~~~~~~~~

* Encrypted IP does not work: yavs does not decrypt it (``E0012``).
* Xilinx ``UNISIM``, ``UNIMACRO`` and ``UNIFAST`` work; the ``secureip`` primitives and XPM macros cannot, and Intel
  and Lattice libraries are not done: :doc:`vendor-libraries`.
* No precompiled libraries are shipped. You analyze libraries from their sources (:doc:`libraries`).

Simulation
~~~~~~~~~~

* No timing simulation: no SDF file is read, and vendor timing models run with zero delay.
* Threads help only some designs (:doc:`threads`).

Tools
~~~~~

* A debugger session runs the interpreter on one thread, and it cannot be combined with ``--cov`` or ``--profile``.
* Breakpoint conditions cannot call functions or use attributes.
* VHPI is partial: :doc:`vhpi` has the table.
* There is no graphical interface of its own: use VS Code for debugging and a waveform viewer for FST.

See also
--------

* :doc:`troubleshooting`: common problems and what to do.
* :doc:`language-support`: the state of each VHDL feature.
* :doc:`conformance`: what is checked, and how.
