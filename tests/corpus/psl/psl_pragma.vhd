-- difftest: top=psl_pragma std=2008 exit=1 ignore=path psl
-- The PSL subset in comment pragmas (#226), one statement over three lines.
-- ignore=path: for a PSL directive the reference prints its label, not a process path.
entity psl_pragma is end;
architecture t of psl_pragma is
  signal clk : bit := '0';
  signal req, ack : bit := '0';
  signal n : natural := 0;
  -- psl default clock is clk'event and clk = '1';
begin
  clk <= not clk after 5 ns when now < 120 ns;
  process (clk) begin
    if clk'event and clk = '1' then
      n <= n + 1;
      req <= '0';
      ack <= '0';
      if n = 1 or n = 5 then req <= '1'; end if;
      if n = 2 then ack <= '1'; end if;
    end if;
  end process;
  -- psl handshake: assert always
  --   req -> next ack
  --   report "no ack";
  -- psl window: assert never n = 9 report "n reached 9";
  -- psl seen: cover {ack} report "ack seen";
end;
