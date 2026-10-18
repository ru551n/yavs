-- yavs-test: std=2008
-- expect: E1811 9:12 `lp` is not a label of an enclosing loop
-- LRM 10.11
entity e is end entity;
architecture a of e is
begin
  process begin
    for i in 0 to 1 loop
      exit lp;
    end loop;
    wait;
  end process;
end architecture;
