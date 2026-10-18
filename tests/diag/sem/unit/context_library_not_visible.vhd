-- yavs-test: std=2008
-- expect: E1100 9:19 `ieee` is not declared
-- LRM 13.4: a context that does not name the library does not make it visible
context c is
  library std;
end context;
entity e is end entity;
context work.c;
architecture a of ieee.e is
begin
end architecture;
