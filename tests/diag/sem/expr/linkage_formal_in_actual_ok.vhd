-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.6.3: a `linkage` formal takes an actual of any mode, an `in` port included.
entity c is port (l : linkage integer); end;
architecture a of c is begin end;
entity top is port (p1 : in integer); end;
architecture a of top is
  component cc port (l : linkage integer); end component;
begin
  u  : cc port map (p1);
  u2 : entity work.c port map (l => p1);
end;
