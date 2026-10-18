-- yavs-test: std=2008
-- expect: E1360 10:5 `force` target `v` must be a signal
-- LRM 10.5.2
entity e is end entity;
architecture a of e is
begin
  process
    variable v : bit;
  begin
    v <= force '1';
    wait;
  end process;
end architecture;
