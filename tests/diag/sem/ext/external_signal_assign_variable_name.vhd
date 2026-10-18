-- yavs-test: std=2008
-- expect: E1601 8:5 external name of class `variable` cannot be the target of a signal assignment
-- LRM 8.7
entity e is end entity;
architecture a of e is
begin
  process begin
    <<variable .e.v : integer>> <= 1;
    wait;
  end process;
end architecture;
