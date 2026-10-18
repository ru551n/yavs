-- yavs-test: std=2008
-- expect: E1101 10:14 `y` is already declared in this region
-- LRM 12.3
entity e is end entity;
architecture a of e is
  signal x : integer;
begin
  process
    variable y : integer;
    variable y : bit;
  begin
    wait;
  end process;
end architecture;
