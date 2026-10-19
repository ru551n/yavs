-- yavs-test: std=2008
-- expect: W1501 7:14 generic `n` of a local generic package is never used
-- LRM 4.7
entity e is end;
architecture a of e is
  package g is
    generic (n : integer);
    constant c : integer := 1;
  end package;
begin
end;
