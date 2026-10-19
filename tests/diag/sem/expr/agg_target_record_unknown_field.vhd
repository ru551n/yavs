-- yavs-test: std=2008
-- expect: E1100 13:6 no element `c`
-- LRM 10.6.2.1 (#350)
-- A field name in an aggregate target must be a field of the right-hand side's record type.
entity e is end entity;
architecture a of e is
  type rec is record a, b : integer; end record;
  function f return rec is begin return (1, 2); end function;
begin
  process
    variable x, y : integer;
  begin
    (c => x, b => y) := f;
    wait;
  end process;
end architecture;
