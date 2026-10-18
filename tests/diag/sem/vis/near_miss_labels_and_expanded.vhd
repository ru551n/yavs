-- yavs-test: std=2008
-- expect: none
-- LRM 8.3
entity e is end entity;
architecture a of e is
  signal s : integer;
begin
  p1 : process
    variable v : integer := 0;
  begin
    p1.v := 1;
    wait;
  end process;
end architecture;
