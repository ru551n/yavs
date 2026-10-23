-- yavs-test: std=2008
-- expect: none
-- LRM 12.3 (a type declared in the entity under the name of the implicit library `work`
-- hides the library name in the architecture; #640)
entity e is
  type work is (foo, bar);
end entity;
architecture a of e is
begin
  process
    variable v : work := foo;
  begin
    wait;
  end process;
end architecture;
