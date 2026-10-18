-- yavs-test: std=2008
-- expect: none
-- LRM 13.4: a context reference makes the library clause of the context declaration visible
context c is
  library ieee;
  use ieee.std_logic_1164.all;
end context;
context work.c;
entity e is
  port (s : in std_logic);
end entity;
