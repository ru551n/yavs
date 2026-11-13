-- difftest: top=inout_port_collapse_force std=2008 ref=hand ignore=order
-- Forces around `inout` ports: a `force out` of a formal (its driving value, so the actual
-- resolves the forced value with its other source), a `force in` of a formal (its effective
-- value only, the actual keeps the resolved drivers), a `force` of the actual of a third
-- formal, each in an instance of its own entity next to an instance of a plain entity on
-- another actual that nothing forces; the values seen through each formal and actual.
-- ref=hand: the second reference simulator does not parse force modes; the first changes the
-- actual on `force in` of the formal (c=L from 2 ns): LRM 10.5.2.1 forces the effective value
-- of the port only, as for `in` ports (catalogue row `force-in-port-actual`). The rest is the
-- first reference simulator's output. ignore=order: readers report in the same delta.
library ieee;
use ieee.std_logic_1164.all;

entity iopf_plain is
  port (p : inout std_logic; tag : in natural);
end;
architecture t of iopf_plain is
begin
  drv : p <= '1', 'Z' after 4 ns, '0' after 8 ns;
  rd : process (p)
  begin
    report "plain" & integer'image(tag) & " p=" & to_string(p);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity iopf_fout is
  port (p : inout std_logic);
end;
architecture t of iopf_fout is
begin
  drv : p <= '1', 'Z' after 4 ns, '0' after 8 ns;
  frc : process
  begin
    wait for 2 ns;
    p <= force out 'H';
    wait for 4 ns;
    p <= release out;
    wait;
  end process;
  rd : process (p)
  begin
    report "fout p=" & to_string(p);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity iopf_fin is
  port (p : inout std_logic);
end;
architecture t of iopf_fin is
begin
  drv : p <= '1', 'Z' after 4 ns, '0' after 8 ns;
  frc : process
  begin
    wait for 2 ns;
    p <= force in 'L';
    wait for 4 ns;
    p <= release in;
    wait;
  end process;
  rd : process (p)
  begin
    report "fin p=" & to_string(p);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity inout_port_collapse_force is end;
architecture t of inout_port_collapse_force is
  signal a, b, c, d, e : std_logic;
begin
  u1 : entity work.iopf_plain port map (a, 1);
  u2 : entity work.iopf_fout port map (b);
  u3 : entity work.iopf_fin port map (c);
  u4 : entity work.iopf_plain port map (d, 4);
  u5 : entity work.iopf_plain port map (e, 5);
  drv_a : a <= 'Z', 'L' after 6 ns;
  drv_b : b <= 'Z', 'L' after 6 ns;
  drv_c : c <= 'Z', 'L' after 6 ns;
  drv_d : d <= 'Z', 'L' after 6 ns;
  drv_e : e <= 'Z', 'L' after 6 ns;
  frc : process
  begin
    wait for 3 ns;
    d <= force 'X';
    wait for 4 ns;
    d <= release;
    wait;
  end process;
  rd : process (a, b, c, d, e)
  begin
    report "top a=" & to_string(a) & " b=" & to_string(b) & " c=" & to_string(c) & " d="
      & to_string(d) & " e=" & to_string(e);
  end process;
end;
