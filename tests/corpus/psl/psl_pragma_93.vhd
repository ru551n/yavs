-- difftest: top=psl_pragma_93 std=1993 exit=1 ignore=path psl
-- `-- psl` comments in a 1993 file: the PSL words are reserved in the pragma (#841).
library ieee; use ieee.std_logic_1164.all;
entity psl_pragma_93 is end;
architecture t of psl_pragma_93 is
  signal clk : std_logic := '0';
  signal a : bit := '0';
  signal d : bit;
begin
  -- psl default clock is rising_edge(clk);
  -- psl assert always a = '1' report "always";
  -- psl assert never { a = '0' && d = '1' } report "never";
  clk <= not clk after 5 ns when now < 30 ns else clk;
  d <= '1' after 12 ns;
end;
