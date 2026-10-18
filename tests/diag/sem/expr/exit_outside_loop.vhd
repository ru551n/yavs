-- yavs-test: std=2008
-- expect: E1811 8:5 `exit` is not inside a loop
-- LRM 10.11
entity e is end entity;
architecture a of e is
begin
  process begin
    exit;
    wait;
  end process;
end architecture;
