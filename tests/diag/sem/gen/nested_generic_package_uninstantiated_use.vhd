-- yavs-test: std=2008
-- expect: E1505 10:27 local generic package used without an instance
-- LRM 4.9
entity e is end;
architecture a of e is
  package g is
    generic (n : integer);
    constant c : integer := n;
  end package;
  constant k : integer := g.c;
begin
end;
