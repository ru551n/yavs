-- A loop without a wait whose jump back is reached from an `if` without an else: the false edge
-- goes straight to the head of the loop, so the jump back must still be counted (#1087). With
-- last = 0 the count is never reached, so the loop never ends by itself.
use std.env.all;
entity stuck_loop is
  generic (last : natural := 0);
end entity;
architecture a of stuck_loop is
begin
  p : process
    variable n : natural := 0;
  begin
    loop
      n := n + 1;
      if n = last then
        report "reached " & integer'image(n);
        finish;
      end if;
    end loop;
  end process;
end architecture;
