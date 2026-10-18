-- yavs-test: std=2008
-- expect: E1224 8:14 constant `k` needs an initial value
-- expect: E1224 14:14 constant `m` needs an initial value
-- LRM 6.4.2.2
entity e is end entity;
architecture a of e is
  procedure p is
    constant k : integer range 0 to 3;
  begin
    null;
  end procedure;
begin
  process
    constant m : integer;
  begin
    wait;
  end process;
end architecture;
