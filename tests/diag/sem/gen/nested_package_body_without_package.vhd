-- yavs-test: std=2008
-- expect: E1004 6:16 no package `q` in this region
-- LRM 4.8
entity e is end;
architecture a of e is
  package body q is
  end package body;
begin
end;
