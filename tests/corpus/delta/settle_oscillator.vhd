-- difftest: top=settle_oscillator std=2008 exit=1 ignore=fatalmsg
-- A true oscillator must still reach the delta limit (#1064): a ring through two processes,
-- and pure readers of two signals that change in alternate deltas (the shape settle-phase
-- deferral would hold back). The limit's report names the processes of the last delta.
-- The two simulators word the limit differently (ignore=fatalmsg).
entity settle_oscillator is end;
architecture t of settle_oscillator is
  signal go, a, b, c, y, z : bit := '0';
begin
  go <= '1' after 1 ns;
  ring1 : process (a, go) begin b <= go and not a; end process;
  ring2 : process (b) begin a <= b; end process;
  late : process (a) begin c <= a; end process;
  both : process (a, c) begin y <= a xor c; end process;
  tail : process (y) begin z <= not y; end process;
end;
