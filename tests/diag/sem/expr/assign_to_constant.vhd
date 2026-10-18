-- yavs-test: std=2008
-- expect: E1360 10:5 cannot assign to constant `k`
-- LRM 10.6
entity e is end entity;
architecture a of e is
begin
  process
    constant k : integer := 1;
  begin
    k := 2;
    wait;
  end process;
end architecture;
