-- difftest: top=port_individual_unconstrained std=2008
-- Unconstrained array ports associated element by element (#74): the bounds run from the
-- lowest to the highest index named, in the index subtype's direction (as sim1 does).
library ieee; use ieee.std_logic_1164.all;
entity port_individual_unconstrained_sub is port (p : in bit_vector; q : out std_logic_vector); end;
architecture a of port_individual_unconstrained_sub is begin
  process begin
    report "p'left=" & integer'image(p'left) & " p'right=" & integer'image(p'right)
      & " q'left=" & integer'image(q'left) & " q'right=" & integer'image(q'right) & " asc=" & boolean'image(q'ascending);
    wait;
  end process;
  q <= "10" & to_stdlogicvector(p) after 1 ns;
end;
library ieee; use ieee.std_logic_1164.all;
entity port_individual_unconstrained is end;
architecture t of port_individual_unconstrained is
  signal a, b : bit := '1';
  signal x, y : std_logic_vector(1 downto 0);
begin
  u : entity work.port_individual_unconstrained_sub port map (p(0) => a, p(1) => b, q(3) => x(1), q(2) => x(0), q(1) => y(1), q(0) => y(0));
  process begin
    b <= '0'; wait for 2 ns;
    report "x=" & to_string(x) & " y=" & to_string(y);
    wait;
  end process;
end;
