-- difftest: top=psl_builtins std=2008 exit=1 ignore=path psl
-- Built-in functions of the checker subset: prev, stable, rose, fell, onehot, onehot0,
-- countones, inside asserts, implications and covers. At the first tick prev(x) is x itself.
-- ignore=path: for a PSL directive the reference prints its label, not a process path.
entity psl_builtins is end;
architecture t of psl_builtins is
  signal clk : bit := '0';
  signal a : bit := '1';
  signal b : bit := '0';
  signal v : bit_vector(3 downto 0) := "0000";
  -- psl default clock is clk'event and clk = '1';
begin
  clk <= not clk after 5 ns when now < 90 ns;
  a <= '1', '0' after 22 ns, '1' after 42 ns;
  b <= '1' after 12 ns, '0' after 32 ns;
  v <= "0000", "0001" after 12 ns, "0011" after 22 ns, "0100" after 42 ns, "1111" after 62 ns;
  -- psl rose_a: assert never rose(a) report "rose a";
  -- psl fell_a: assert never fell(a) report "fell a";
  -- psl rose_b: assert never rose(b) report "rose b";
  -- psl stable_a: assert never stable(a) report "stable a";
  -- psl prev_a: assert never (prev(a) = '0') report "prev a is 0";
  -- psl prev_v2: assert never (prev(v, 2) = "0011") report "prev2 v is 0011";
  -- psl one: assert never onehot(v) report "onehot v";
  -- psl zero: assert never onehot0(v) report "onehot0 v";
  -- psl two: assert never (countones(v) = 2) report "two ones";
  -- psl four: assert always (countones(v) < 4) report "four ones";
  -- psl imp: assert always (rose(b) -> next (stable(a))) report "a changed after b rose";
  -- psl cov: cover {fell(b)} report "fell b";
end;
