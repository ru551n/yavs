-- difftest: top=wait_in_sensitivity_proc exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references word it
-- differently).
-- A procedure called by a process with a sensitivity list shall not execute a wait
-- (LRM 11.3); the procedure is fine for a process without one (#379).
-- (ignore=fatalmsg: the references word the error differently and name the process.)
-- The procedure is in a package, so the analysis cannot see its wait: the error is raised when
-- the wait executes (a procedure in the same unit is rejected at analysis, #597).
package wait_in_sensitivity_pkg is
  procedure pause;
end package;
package body wait_in_sensitivity_pkg is
  procedure pause is
  begin
    wait for 1 ns;
  end procedure;
end package body;

use work.wait_in_sensitivity_pkg.all;
entity wait_in_sensitivity_proc is end;
architecture a of wait_in_sensitivity_proc is
  signal s : bit;
begin
  s <= '1' after 5 ns;
  ok : process
  begin
    pause;
    report "ok after pause";
    wait;
  end process;
  bad : process (s)
  begin
    if now > 0 ns then
      pause;
      report "not reached";
    end if;
  end process;
end;
