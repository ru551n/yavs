-- yavs-test: std=2008
-- expect: E1340 10:11 condition must be BOOLEAN, found `integer`
-- LRM 10.10
entity e is end entity;
architecture a of e is
begin
  process
    variable n : integer;
  begin
    while n loop
      null;
    end loop;
    wait;
  end process;
end architecture;
