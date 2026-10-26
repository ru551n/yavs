-- difftest: top=slice_loop_bounds std=2008 exit=1 ignore=fatalmsg
-- (The error text differs between simulators: only that it is a fatal error is compared.)
-- Slices `v (4 * i to 4 * i + 3)` copied and tested in a loop (the digit loop of a textio HRead):
-- in range they are plain copies, the first slice past the end of the vector is the error (#771).
library ieee; use ieee.std_logic_1164.all;
entity slice_loop_bounds is end;
architecture a of slice_loop_bounds is
begin
  process
    variable v : std_ulogic_vector(0 to 15) := (others => '0');
    variable n : integer := 4;
    variable bad : boolean := false;
  begin
    for i in 0 to n - 1 loop
      v(4 * i to 4 * i + 3) := "1010";
      if is_x(v(4 * i to 4 * i + 3)) then
        bad := true;
      end if;
    end loop;
    assert not bad and v = "1010101010101010" report "plain" severity failure;
    for i in 1 to n - 1 loop
      v(4 * i to 4 * i + 3) := v(4 * (i - 1) to 4 * (i - 1) + 3);
    end loop;
    assert v = "1010101010101010" report "copy" severity failure;
    report "in range";
    v(4 * n to 4 * n + 3) := "0000";
    report "unreachable";
    wait;
  end process;
end;
