-- difftest: top=psl_abort std=2008 exit=1 ignore=path,order
-- abort and async_abort run like sync_abort: the abort condition is checked at the clock
-- tick, and a start whose condition holds is dropped (the LRM defines abort as asynchronous and
-- the references abort between ticks too; no condition here changes between ticks, #636, L133). All forms sit inside always,
-- where the first reference simulator agrees with the PSL definition by prefix.
library ieee;
use ieee.std_logic_1164.all;
entity psl_abort is end;
architecture t of psl_abort is
  signal clk : std_logic := '0';
  signal a, b, c, r : std_logic := '0';
  signal n : integer := 0;
  constant PA : string := "1101001011100110";
  constant PB : string := "0110100110010101";
  constant PC : string := "0011010101001011";
  constant PR : string := "0000100000100001";
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      if n < PA'length then
        a <= '1' when PA(n + 1) = '1' else '0';
        b <= '1' when PB(n + 1) = '1' else '0';
        c <= '1' when PC(n + 1) = '1' else '0';
        r <= '1' when PR(n + 1) = '1' else '0';
      else
        a <= '0'; b <= '0'; c <= '0'; r <= '0';
      end if;
    end if;
  end process;

  plain: assert always (c = '1' -> next[2] (b = '1')) report "plain";
  ab_and: assert always ((c = '1') -> ((next[2] (b = '1')) abort (a = '1' and b = '1'))) report "ab_and";
  aab_and: assert always ((c = '1') -> ((next[2] (b = '1')) async_abort (a = '1' and b = '1'))) report "aab_and";
  ab_rst: assert always ((a = '1') -> ((next[3] (b = '1')) abort (r = '1'))) report "ab_rst";
  aab_rst: assert always ((a = '1') -> ((next[3] (b = '1')) async_abort (r = '1'))) report "aab_rst";
  ab_next: assert always ((c = '1') -> next ((next (b = '1')) abort (a = '1'))) report "ab_next";
end;
