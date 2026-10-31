-- yavs-test: std=1993
-- expect: E1360 10:38 must be a static signal name or a globally static expression
-- expect: E1360 11:38 must be a static signal name or a globally static expression
-- LRM 1993 1.1.1.2 (#926)
entity sb is port (i : in bit); end;
entity top is end;
architecture a of top is
  signal s, t : bit; signal v : bit_vector(0 to 1); signal k : integer := 0;
begin
  u1 : entity work.sb port map (i => s and t);
  u2 : entity work.sb port map (i => v(k));
  u3 : entity work.sb port map (i => v(1));
end;
