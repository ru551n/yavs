-- LRM 10.2, 8.7: process (all) reading an external name directly
entity sub is end;
architecture a of sub is signal n : integer := 0; begin n <= 3 after 1 ns; end;
entity e is end;
architecture t of e is
begin
  u : entity work.sub;
  process (all) begin report "RES n=" & integer'image(<<signal .e.u.n : integer>>); end process;
end;
