-- difftest: top=psl_sequences std=2008 exit=1 ignore=path,order
-- Sequences in cover, never, suffix implication and as properties (IEEE 1850 6.1, 6.2.1.6):
-- concatenation, fusion, alternation, consecutive repetition with counts and ranges, goto
-- repetition, length-matching and. Several directives report at the same tick, hence
-- ignore=order. The stimulus is a fixed bit pattern per signal so that every directive hits.
library ieee;
use ieee.std_logic_1164.all;
entity psl_sequences is end;
architecture t of psl_sequences is
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

  cat2:   cover {a = '1'; b = '1'} report "cat2";
  cat3:   cover {a = '1'; b = '1'; c = '1'} report "cat3";
  fuse1:  cover {a = '1' : b = '1'} report "fuse1";
  fuse2:  cover {{a = '1'; b = '1'} : {b = '1'; c = '1'}} report "fuse2";
  alt:    cover {(a = '1') or (b = '1'); c = '1'} report "alt";
  rep2:   cover {a = '1'[*2]} report "rep2";
  reprng: cover {a = '1'[*1 to 3]; b = '1'} report "reprng";
  star:   cover {a = '1'[*]; b = '1'} report "star";
  plus:   cover {a = '1'[+]; b = '1'} report "plus";
  goto1:  cover {a = '1'[->2]} report "goto1";
  goto2:  cover {a = '1'[->2]; b = '1'} report "goto2";
  seqrep: cover {{a = '1'; b = '1'}[*2]} report "seqrep";
  both:   cover {{a = '1'; b = '1'} && {c = '1'; c = '1'}} report "both";
  nev1:   assert never {a = '1'; b = '1'} report "never ab";
  nev2:   assert never {a = '1'[*2]; b = '1'} report "never aab";
  imp1:   assert always {a = '1'; b = '1'} |-> c = '0' report "imp overlap";
  imp2:   assert always {a = '1'; b = '1'} |=> c = '1' report "imp next";
  imp3:   assert always {a = '1'} |=> {b = '1'; c = '1'} report "imp seq";
  imp4:   assert always {a = '1'} |-> {b = '1'; c = '1'} report "imp seq overlap";
  imp5:   assert always {a = '1'; a = '1'} |=> b = '1' report "imp rep";
  imp6:   assert always {a = '1'} |=> {b = '1'[*2]} report "imp rhs rep";
  imp7:   assert always {a = '1'} |=> {b = '1'[*0 to 2]} report "imp rhs nullable range";
  imp8:   assert always {a = '1'} |=> {b = '1'[*]} report "imp rhs star";
  seq1:   assert always {a = '1'; b = '1'} report "always seq";
  seq2:   assert {a = '0'; b = '1'} report "first only";
end;
