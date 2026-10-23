-- yavs-test: std=2019
-- expect: E1507 10:31 the formal `q` has an anonymous type and cannot be converted
-- LRM 2019 6.5.3: the type of such a port is the type of its actual, which a conversion of the formal hides (#606)
entity e is port (q : out type is range <>); end entity;
architecture a of e is begin end architecture;
entity top is end;
architecture a of top is
  signal s : integer;
begin
  u : entity work.e port map (integer(q) => s);
end architecture;
