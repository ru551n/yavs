-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.6.3: an `out` formal may drive a signal or an `inout` port; an `in` actual is read.
entity c is port (o : out integer; i : in integer); end;
architecture a of c is begin o <= i; end;
entity top is port (p1 : in integer; p2 : inout integer); end;
architecture a of top is
  signal s : integer;
  component cc port (o : out integer; i : in integer); end component;
begin
  u  : cc port map (s, p1);
  u2 : entity work.c port map (o => p2, i => p1);
end;
