-- difftest: top=psl_subset std=2008 exit=1 ignore=path
-- The PSL subset (#226): every directive is checked on its clock. ignore=path: for a PSL
-- directive the reference prints its label and source line, not a process path.
library ieee;
use ieee.std_logic_1164.all;
entity psl_subset is end;
architecture t of psl_subset is
  signal clk : std_logic := '0';
  signal a, b : std_logic := '0';
  signal n : integer := 0;
  signal flag : boolean := false;
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      a <= '0';
      b <= '0';
      if n = 2 or n = 10 then a <= '1'; end if;
      if n = 4 or n = 13 then b <= '1'; end if;
      flag <= n = 15;
    end if;
  end process;
  p1: assert always n < 18 report "n too big: " & integer'image(n);
  p2: assert never (n = 3);
  p3: assert always (a -> next b);
  p4: assert always a -> next[2] (b) report "next2 failed";
  p5: assert never flag report "flag set";
  p6: assert always (n /= 7);
  c1: cover {n = 8} report "eight";
  c2: cover {a = '1'};
end;
