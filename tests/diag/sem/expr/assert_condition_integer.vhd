-- yavs-test: std=2008
-- expect: E1340 8:12 condition must be BOOLEAN, found `integer`
-- LRM 10.3
entity e is end entity;
architecture a of e is
begin
  process begin
    assert 1 report "x";
    wait;
  end process;
end architecture;
