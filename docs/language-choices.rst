Where simulators differ
=======================

The VHDL standard leaves some points open, and simulators do not always agree on them. Where the
standard is clear, yavs follows it and reports a violation early; where it is open, yavs is
permissive. These are the differences you are most likely to run into when you move a design to
yavs, with what to do about each.

.. list-table::
   :header-rows: 1
   :widths: 30 40 30

   * - Situation
     - What yavs does
     - What you can do
   * - A testbench depends on which process runs first in a cycle
     - Runs processes in one fixed order, the same at any ``--threads``.
     - ``--shuffle[=SEED]`` runs another order to find the dependency; pass values through
       signals instead.
   * - Verification code with a shared variable that is not protected, a pure function that calls
       an impure one, or an ``others`` aggregate assigned to an unconstrained object
     - Errors, as the standard says.
     - ``--relaxed`` on ``analyze`` and ``run`` turns them into warnings.
   * - PSL in ``-- psl`` comments
     - Read only when asked.
     - ``--psl`` on ``analyze`` and ``run``.
   * - A TIME literal with a unit below ``--resolution`` (``1 ps`` with ``--resolution=ns``)
     - Error before time 0, also in code that never runs.
     - Use a finer ``--resolution`` (default ``fs``) or write times in coarser units.
   * - Files of different standards in one design
     - Allowed; the design uses the packages of the newest standard.
     - Keep VHDL-1993 code that overloads on both ``std_logic_vector`` and ``std_ulogic_vector``
       in a design of 1993 files only.
   * - VHDL-1993 code that relies on rules VHDL-2008 relaxed
     - Error ``E1007`` under ``--std=93``, naming the 1993 rule.
     - Analyze the file with ``--std=2008``.
   * - A static index out of range or a constant overflow in code that may be dead
     - Warning (``W1404``, ``W1401``); the check still runs if the code runs.
     - ``-Werror=W1404`` makes it an error.
   * - An external name as the actual of an ``in`` port
     - The port follows the signal in the same delta cycle.
     - Do not depend on the delta in which such a port changes.
   * - ``std_logic_arith``, ``std_logic_unsigned`` and the other Synopsys packages
     - Built into ``ieee`` and visible without a flag.
     - Nothing; prefer ``numeric_std`` in new code.

The complete list of choices, with the sections of the standard and portable forms, is kept in the
repository: `docs/design/language-choices.rst
<https://github.com/ru551n/yavs/blob/main/docs/design/language-choices.rst>`__.
