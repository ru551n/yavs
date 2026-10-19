-- difftest: top=elab_report ref=hand
-- ref=hand: the reference simulator prints source traces after the message; the harness runs
-- `elab` and `run`, each elaborating the design, so the message appears once per command.
-- A report in a function called during elaboration (LRM 10.4) prints its message and
-- elaboration goes on.
package elab_report_pkg is
  impure function f return integer;
end package;

package body elab_report_pkg is
  impure function f return integer is
  begin
    report "hello from f";
    return 1;
  end function;
end package body;

use work.elab_report_pkg.all;

entity elab_report is
end entity;

architecture a of elab_report is
  signal s : integer := f;
begin
  process
  begin
    assert s = 1 severity failure;
    report "s = " & integer'image(s);
    wait;
  end process;
end architecture;
