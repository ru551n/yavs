-- A procedure containing a wait, in a package, called by a process with a sensitivity list
-- (LRM 11.3): the call is not executed.
package pw is
  procedure pause;
end package;
package body pw is
  procedure pause is
  begin
    wait for 1 ns;
  end procedure;
end package body;

use work.pw.all;
entity top is end;
architecture a of top is
  signal s : bit;
begin
  process (s)
  begin
    if now > 0 ns then
      pause;
    end if;
    report "after";
  end process;
  s <= '1' after 2 ns;
end;
