-- yavs-test: std=2008
-- expect: E1360 9:5 cannot assign to constant `c`
-- LRM 10.5
entity e is end entity;
architecture a of e is
  constant c : integer := 1;
begin
  process begin
    c <= 2;
    wait;
  end process;
end architecture;
