-- A process without a wait statement that counts its jumps back and ends the run with finish
-- after `last` of them: legitimate while `last` stays below the bound on an activation (#583).
use std.env.all;
entity stuck_count is
  generic (last : natural := 3000000);
end entity;
architecture a of stuck_count is
  signal a : bit := '0';
begin
  p : process
    variable n : natural := 0;
  begin
    a <= not a;
    n := n + 1;
    if n = last then
      report "counted " & integer'image(n);
      finish;
    end if;
  end process;
end architecture;
