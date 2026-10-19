-- yavs-test: std=2008
-- expect: E1320 13:25 the type of this aggregate cannot be determined
-- LRM 10.5.2.1, 10.6.2.1 (#350)
-- An aggregate target takes its type from the right-hand side; when that is an aggregate
-- too, nothing gives a type to either.
entity e is end entity;
architecture a of e is
  type rec is record a, b : integer; end record;
begin
  process
    variable x, y : integer;
  begin
    (a => x, b => y) := (1, 2);
    wait;
  end process;
end architecture;
