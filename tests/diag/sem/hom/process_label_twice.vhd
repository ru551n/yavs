-- yavs-test: std=2008
-- expect: E1101 8:3 label `l` is already declared in this region
-- LRM 12.3
entity e is end entity;
architecture a of e is
begin
  l : process begin wait; end process;
  l : process begin wait; end process;
end architecture;
