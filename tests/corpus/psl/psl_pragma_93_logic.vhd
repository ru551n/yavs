-- difftest: top=psl_pragma_93_logic std=1993 exit=1 ignore=path psl
-- Booleans of type std_ulogic and bit in directives of a 1993 file, which has no condition operator (#911).
library ieee; use ieee.std_logic_1164.all;
entity psl_pragma_93_logic is end;
architecture t of psl_pragma_93_logic is
  signal clk : std_logic := '0';
  signal x : std_logic := '0';
  signal b : bit := '1';
begin
  -- psl default clock is rising_edge(clk);
  -- psl assert always x report "std_ulogic";
  -- psl assert never b report "bit";
  clk <= not clk after 5 ns when now < 20 ns else clk;
  x <= '1' after 12 ns;
  b <= '0' after 12 ns;
end;
