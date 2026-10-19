-- difftest: top=psl_clock std=2008 exit=1 ignore=path ref=hand
-- The PSL subset (#226): explicit `@` clocks, `assume`, `severity`, `next[0]`. ref=hand: the first
-- reference simulator rejects or crashes on explicit clocks and `assume`; this expected output
-- is the second reference simulator's, in canonical form. ignore=path: for a PSL directive the
-- references print its label or source line, not a process path.
entity psl_clock is end;
architecture t of psl_clock is
  signal clk : bit := '0';
  signal n : integer := 0;
begin
  clk <= not clk after 5 ns when now < 100 ns;
  n <= n + 1 when clk'event and clk = '1';
  rise: assert always n < 7 @rising_edge(clk) report "rise " & integer'image(n)
    severity warning;
  fall: assert always (n = 3 -> n > 5) @(clk'event and clk = '0') report "fall";
  hold: assume (always n /= 2) @rising_edge(clk);
  same: assert (always (n = 5 -> next[0] (n < 5))) @rising_edge(clk) report "next0";
end;
