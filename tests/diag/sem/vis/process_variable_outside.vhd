-- yavs-test: std=2008
-- expect: E1100 15:10 `v` is not declared
-- LRM 12.2
entity e is end entity;
architecture a of e is
begin
  p1 : process
    variable v : integer := 0;
  begin
    wait;
  end process;
  p2 : process
    variable w : integer;
  begin
    w := v;
    wait;
  end process;
end architecture;
