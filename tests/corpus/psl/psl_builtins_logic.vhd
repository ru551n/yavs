-- difftest: top=psl_builtins_logic std=2008 exit=1 ignore=path psl ref=hand
-- Built-in functions of the checker subset on std_logic signals, with unknown values:
-- an edge needs a known value on both sides (rose: '1' after '0', fell: '0' after '1'),
-- stable compares with `=`, isunknown is true for any of U X Z W -, a vector counts its '1'
-- elements. ref=hand: the reference simulators do not run all of it (one stops on std_logic
-- edges, one lacks isunknown and countones); the expected output is what the one that runs
-- it all prints, with the order of messages at one time as yavs gives it.
library ieee;
use ieee.std_logic_1164.all;
entity psl_builtins_logic is end;
architecture t of psl_builtins_logic is
  signal clk : std_logic := '0';
  signal a : std_logic := '1';
  signal b : bit := '0';
  signal v : std_logic_vector(3 downto 0) := "0000";
  -- psl default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 130 ns;
  a <= '1', '0' after 22 ns, '1' after 42 ns, 'X' after 62 ns, '1' after 82 ns;
  b <= '1' after 12 ns, '0' after 32 ns;
  v <= "0000", "0001" after 12 ns, "0011" after 22 ns, "01X0" after 42 ns, "0100" after 62 ns, "0000" after 82 ns, "1111" after 102 ns;
  -- psl cr_a: assert never (rose(a)) report "rose a";
  -- psl cf_a: assert never (fell(a)) report "fell a";
  -- psl cr_b: assert never (rose(b)) report "rose b";
  -- psl cf_b: assert never (fell(b)) report "fell b";
  -- psl cs_a: assert never (stable(a)) report "stable a";
  -- psl cu_v: assert never (isunknown(v)) report "v unknown";
  -- psl co1: assert never (onehot(v)) report "onehot v";
  -- psl co0: assert never (onehot0(v)) report "onehot0 v";
  -- psl cc2: assert never (countones(v) = 2) report "two ones";
  -- psl cc4: assert never (countones(v) = 4) report "four ones";
  -- psl cs_v: assert never (stable(v)) report "stable v";
  -- psl cp_v: assert never (prev(v) = "0011") report "prev v is 0011";
end;
