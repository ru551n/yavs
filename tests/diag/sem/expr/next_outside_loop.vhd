-- yavs-test: std=2008
-- expect: E1811 8:5 `next` is not inside a loop
-- LRM 10.10
entity e is end entity;
architecture a of e is
begin
  process begin
    next;
    wait;
  end process;
end architecture;
