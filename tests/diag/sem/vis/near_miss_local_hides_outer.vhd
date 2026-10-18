-- yavs-test: std=2008
-- expect: none
-- LRM 12.3
entity e is end entity;
architecture a of e is
  signal x : integer := 0;
begin
  process
    variable x : integer := 1;
  begin
    x := x + 1;
    wait;
  end process;
end architecture;
