-- yavs-test: std=2008
-- expect: E1802 8:5 `return` is only allowed in a subprogram
-- LRM 10.13
entity e is end entity;
architecture a of e is
begin
  process begin
    return;
  end process;
end architecture;
