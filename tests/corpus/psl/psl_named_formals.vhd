-- difftest: top=psl_named_formals std=2008 ref=hand exit=1 ignore=path,order
-- ref=hand: no reference simulator takes `const` or `sequence` formals; the result is that of
-- the same directives written out, which both run (IEEE 1850 6.5.2). A sequence used for two
-- formals, a formal handed on to another declaration, a count from a formal and property formals.
library ieee;
use ieee.std_logic_1164.all;
entity psl_named_formals is end;
architecture t of psl_named_formals is
  signal clk : std_logic := '0';
  signal a, b, c : std_logic := '0';
  signal n : integer := 0;
  constant PA : string := "1101001011100110";
  constant PB : string := "0110100110010101";
  constant PC : string := "0011010101001011";
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
      else
        a <= '0'; b <= '0'; c <= '0';
      end if;
    end if;
  end process;

  sequence rep (boolean x; const k) is {x[*k]; c = '1'};
  sequence both (sequence p, q) is {p; q};
  sequence twice (sequence p) is both(p, p);
  sequence upto (boolean x; const lo, hi) is {x[*lo to hi]; b = '1'};
  property then_y (sequence p, boolean y) is always (p |=> y);
  property whole (property q) is always q;
  property within_next (property q, boolean x) is always (x -> next q);

  c1: cover rep(a = '1', 2) report "rep 2";
  c2: cover rep(b = '1', 1) report "rep 1";
  c3: cover both({a = '1'; b = '1'}, {c = '1'}) report "both";
  c4: cover twice({a = '1'; b = '1'}) report "twice";
  c5: cover upto(a = '1', 1, 3) report "upto";
  c6: cover {both(a = '1', b = '1'); c = '1'} report "nested both";
  p1: assert then_y({a = '1'; b = '1'}, c = '1') report "after";
  p2: assert whole(a = '1' -> next[2] (c = '1')) report "whole";
  p3: assert whole({a = '1'; b = '1'} |=> next_e[1 to 2] (c = '1')) report "property formal";
  p4: assert within_next((b = '1' until c = '1'), a = '1') report "nested formals";
end;
