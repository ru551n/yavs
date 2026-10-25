-- yavs-test: std=2008
-- expect: E1360 12:21 p1 with mode in cannot be associated with formal o with mode out
-- expect: E1360 13:37 p1 with mode in cannot be associated with formal o with mode out
-- LRM 6.5.6.3: an `in` port as the actual of an `out` formal is a mode error, in the
-- component form as in the entity form (#654).
entity c is port (o : out integer; i : in integer); end;
architecture a of c is begin o <= i; end;
entity top is port (p1 : in integer; p2 : in integer); end;
architecture a of top is
  component cc port (o : out integer; i : in integer); end component;
begin
  u  : cc port map (p1, p2);
  u2 : entity work.c port map (o => p1, i => p2);
end;
