-- yavs-test: std=2008
-- expect: E1002 9:19 design unit `e` not found in library `ieee`
-- LRM 3.3.1, 13.4: `ieee` is visible through the context reference, so the library is found
context c is
  library ieee;
end context;
entity e is end entity;
context work.c;
architecture a of ieee.e is
begin
end architecture;
