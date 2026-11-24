-- difftest: top=generic_delay
-- Scalar assignments whose delay comes from a generic below the top: a real generic times a time
-- in a constant, an integer generic times a time, a time generic itself, in processes with a
-- sensitivity list and in concurrent statements. Each line toggles with its own period; the
-- counts of its events in 100 ns are checked (#1198: compiled code drove them with no delay).
library ieee;
use ieee.std_logic_1164.all;

entity generic_delay_gen is
  generic (g : real := 2.5; n : integer := 3; t : time := 4 ns);
  port (a, b, c, d, e : out std_ulogic);
end entity;

architecture a of generic_delay_gen is
  constant dg : time := g * 1 ns;
  constant dn : time := n * 1 ns;
  signal sa, sb, sc, sd, se : std_ulogic := '0';
begin
  process (sa) begin sa <= not sa after dg; end process;
  process (sb) begin sb <= not sb after dn; end process;
  process (sc) begin sc <= not sc after t; end process;
  sd <= not sd after dg;
  se <= not se after t;
  a <= sa;
  b <= sb;
  c <= sc;
  d <= sd;
  e <= se;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity generic_delay is
end entity;

architecture a of generic_delay is
  type counts_t is array (0 to 4) of natural;
  signal a, b, c, d, e : std_ulogic;
begin

  u : entity work.generic_delay_gen
    generic map (g => 2.5, n => 3, t => 4 ns)
    port map (a, b, c, d, e);

  main : process
    variable k : counts_t := (others => 0);
  begin
    loop
      wait on a, b, c, d, e for 100 ns - now;
      exit when now >= 100 ns;
      next when now = 0 ns; -- the start values reaching the ports
      if a'event then k(0) := k(0) + 1; end if;
      if b'event then k(1) := k(1) + 1; end if;
      if c'event then k(2) := k(2) + 1; end if;
      if d'event then k(3) := k(3) + 1; end if;
      if e'event then k(4) := k(4) + 1; end if;
    end loop;
    report "events " & integer'image(k(0)) & " " & integer'image(k(1)) & " " & integer'image(k(2))
      & " " & integer'image(k(3)) & " " & integer'image(k(4));
    assert k = (39, 33, 24, 39, 24) report "a delay from a generic was lost" severity failure;
    std.env.finish;
  end process;

end architecture;
