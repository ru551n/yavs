-- yavs-test: std=2008
-- expect: E1601 8:5 external name of class `constant` cannot be an assignment target
-- LRM 8.7
entity e is end entity;
architecture a of e is
begin
  process begin
    <<constant .e.k : integer>> <= 1;
    wait;
  end process;
end architecture;
