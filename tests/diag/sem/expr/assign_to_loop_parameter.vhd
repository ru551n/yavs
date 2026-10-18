-- yavs-test: std=2008
-- expect: E1360 9:7 cannot assign to loop parameter `i`
-- LRM 10.10
entity e is end entity;
architecture a of e is
begin
  process begin
    for i in 0 to 3 loop
      i := 1;
    end loop;
    wait;
  end process;
end architecture;
