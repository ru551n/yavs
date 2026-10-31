-- difftest: top=psl_seq_bare_and std=2008 exit=1 ignore=path psl
-- `&&` between booleans of a sequence, with no braces around them (#841).
library ieee; use ieee.std_logic_1164.all;
entity psl_seq_bare_and is end;
architecture t of psl_seq_bare_and is
  signal clk : std_logic := '0';
  signal a : std_logic := '1';
  signal b : std_logic := '1';
  signal c : bit := '0';
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 40 ns;
  b <= '0' after 12 ns;
  c <= '1' after 22 ns;
  assert never { a && b } report "both";
  assert never { a = '1' && c = '1' } report "two";
  assert always { a; a } |=> { a && c = '0' } report "right side";
end;
