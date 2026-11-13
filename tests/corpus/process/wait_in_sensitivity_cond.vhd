-- difftest: top=wait_in_sensitivity_cond exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references word it
-- differently, and one rejects the design at analysis).
-- As `wait_in_sensitivity_proc`, with the wait behind a condition: the procedure runs to its
-- end while the condition is false and the error comes when the wait first executes (LRM 11.3).
package wait_in_sensitivity_cond_pkg is
  procedure work_then_pause(variable r : inout integer; go : boolean);
end package;
package body wait_in_sensitivity_cond_pkg is
  procedure work_then_pause(variable r : inout integer; go : boolean) is
    variable t : integer := 1;
  begin
    for i in 1 to 3 loop
      t := t * 2 + i;
    end loop;
    if go then
      wait for 1 ns;
    end if;
    r := (r + t) mod 100003;
  end procedure;
end package body;

use work.wait_in_sensitivity_cond_pkg.all;
entity wait_in_sensitivity_cond is end;
architecture a of wait_in_sensitivity_cond is
  signal s : integer := 0;
begin
  s <= s + 1 after 5 ns when s < 6;
  p : process (s)
    variable r : integer := 0;
  begin
    work_then_pause(r, s = 4);
    report "s=" & integer'image(s) & " r=" & integer'image(r);
  end process;
end;
