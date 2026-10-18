-- yavs-test: std=2008
-- expect: E1300 8:19 expected `integer`, found `boolean`
-- LRM 5.2.1
entity e is end entity;
architecture a of e is
begin
  process begin
    for i in 0 to true loop
      null;
    end loop;
    wait;
  end process;
end architecture;
