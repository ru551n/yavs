Generics of the top level
=========================

This page says how to set a generic of the top level, from the command line and from VUnit, and how
to write each kind of value.

.. contents:: On this page
   :local:

From the command line
---------------------

``-gNAME=VALUE`` on ``yavs run`` or ``yavs elab``, once per generic:

.. code-block:: bash

    yavs run --work=work:build/work counter_tb -gWIDTH=3 -gCLOCK_PERIOD=2ns

* ``NAME`` is not case sensitive. ``-g NAME=VALUE`` works too.
* The last value of a repeated name wins.
* Only generics of the top level can be set. For a generic deeper in the design, pass it down through
  the top level's generic map.
* Nothing needs to be analyzed again for a new value.

How to write a value
--------------------

The value is written without VHDL quotes around strings and bit strings. A character or an
``std_logic`` keeps its single quotes, so quote it for the shell:

.. list-table::
   :header-rows: 1
   :widths: 30 35 35

   * - Type of the generic
     - Example
     - Note
   * - ``integer``, ``natural``, ``positive``
     - ``-gN=8``, ``-gN=16#ff#``
     - Based literals work.
   * - ``real``
     - ``-gGAIN=2.25``
     -
   * - ``boolean``
     - ``-gFLAG=true``
     - Not case sensitive.
   * - ``time``
     - ``-gPERIOD=5ns``, ``-gPERIOD="5 ns"``
     - A unit is needed.
   * - ``string``
     - ``-gNAME=hello``, ``-gNAME="a b"``
     - Taken as written. ``-gNAME=\"q\"`` is a string with quote characters in it.
   * - ``std_logic_vector``, ``bit_vector``
     - ``-gBITS=1010``
     - One character per element, and as many as the generic has. ``x"A"`` is not accepted.
   * - ``std_logic``, ``character``
     - ``-gONE="'1'"``, ``-gC="'z'"``
     - With the single quotes.

Generics without a default
--------------------------

A generic of the top level with no default and no ``-g`` is error ``E2103``, which says which option
to add. A value that does not fit the type is ``E2104``. A name the top level does not have is warning
``W2105``, and the run goes on; ``-Werror=W2105`` makes it an error, so a misspelt name in a script
cannot go unnoticed:

.. code-block:: text

    $ yavs run --work=work:build/work counter_tb -gNOPE=1
    warning[W2105]: `-gNOPE`: top level `counter_tb` has no generic `NOPE`

Long values
-----------

An argument ``@FILE`` is replaced by the arguments in FILE, one per line, taken as written. Use it for a
long string value, so that it needs no quoting:

.. code-block:: bash

    printf '%s\n' '-gNAME=a long value with "quotes"' > args.txt
    yavs run --work=work:build/work counter_tb @args.txt

From VUnit
----------

A run script sets generics on a testbench or on one configuration of it. VUnit passes each one as
``-gNAME=VALUE``:

.. code-block:: python

    tb_counter = tb.test_bench("tb_counter")
    tb_counter.set_generic("WIDTH", 4)
    for width in (3, 5):
        tb_counter.add_config(name=f"width_{width}", generics={"WIDTH": width})

Each configuration is a test of its own, here ``tb.tb_counter.width_3.counts`` and so on (:doc:`vunit`).

From a debugger launch configuration
------------------------------------

The VS Code extension takes them as ``"generics": { "WIDTH": "8" }`` in ``launch.json``
(:doc:`debugging`).

See also
--------

* :doc:`cli-reference`: ``-g`` on ``run`` and ``elab``.
* :doc:`project-walkthrough`: generics in a small project.
* :doc:`vunit`: configurations in a run script.
