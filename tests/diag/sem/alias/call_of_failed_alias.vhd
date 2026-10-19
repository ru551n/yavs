-- yavs-test: std=2008
-- expect: E1100 9:16 the aliased name is not declared
-- expect: E1100 10:16 the aliased name is not declared
-- LRM 6.6.3, 8.2: calls of the poisoned alias names report nothing
entity e is end entity;
architecture a of e is
begin
  process
    alias s is nonexistent [integer];
    alias f is nonexistent2 [integer return integer];
    variable x : integer;
  begin
    s(1);
    s;
    x := f(1);
    wait;
  end process;
end architecture;
