-- difftest: top=psl_named std=2008 ref=hand exit=1 ignore=path,order
-- ref=hand: the first reference simulator stops on named sequences and properties and the second
-- stops on property formals; the result is that of the same directives written out (checked on both).
-- Declared sequences and properties, with boolean formals, used in covers, as
-- the left and right side of suffix implications and as a whole property (IEEE 1850 6.5).
library ieee;
use ieee.std_logic_1164.all;
entity psl_named is end;
architecture t of psl_named is
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

  sequence ab is {a = '1'; b = '1'};
  sequence abc is {ab; c = '1'};
  sequence pair (boolean x, y) is {x; y};
  sequence rep (boolean x) is {x[*2 to 3]; b = '1'};
  property a_then_b is always (a = '1' -> next b = '1');
  property then_next (boolean x, y) is always (x -> next y);
  property ab_then_c is always (ab |=> c = '1');
  property no_aa (boolean x) is never {x; x};

  c1: cover ab report "ab";
  c2: cover abc report "abc";
  c3: cover pair(b = '1', c = '1') report "pair bc";
  c4: cover {pair(a = '1', a = '0'); b = '1'} report "pair nested";
  c5: cover rep(a = '1') report "rep";
  c6: cover {ab; ab} report "ab ab";
  p1: assert a_then_b report "a then b";
  p2: assert then_next(b = '1', c = '1') report "b then c";
  p3: assert ab_then_c report "ab then c";
  p4: assert no_aa(a = '1') report "no aa";
  p5: assert always ab |-> b = '1' report "ab overlap";
  p6: assert always {a = '1'} |=> pair(b = '1', c = '1') report "rhs pair";
end;
