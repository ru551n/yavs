-- difftest: top=elab_report_error ref=hand exit=1
-- ref=hand: the reference simulator prints source traces after the message; the harness runs
-- `elab` and `run`, each elaborating the design, so the message appears once per command.
-- A report in a function called during elaboration (LRM 10.4) prints its message and
-- elaboration goes on; severity error counts toward the exit status.
package elab_report_error_pkg is
  impure function f return integer;
end package;

package body elab_report_error_pkg is
  impure function f return integer is
  begin
    report "bad from f" severity error;
    return 1;
  end function;
end package body;

use work.elab_report_error_pkg.all;

entity elab_report_error is
end entity;

architecture a of elab_report_error is
  signal s : integer := f;
begin
  process
  begin
    assert s = 1 severity failure;
    report "s = " & integer'image(s);
    wait;
  end process;
end architecture;
