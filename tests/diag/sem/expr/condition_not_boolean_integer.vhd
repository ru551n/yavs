-- yavs-test: std=2008
-- expect: E1340 10:8 condition must be BOOLEAN, found `integer`
-- LRM 10.8
entity e is end entity;
architecture a of e is
begin
  process
    variable i : integer;
  begin
    if i then
      null;
    end if;
    wait;
  end process;
end architecture;
