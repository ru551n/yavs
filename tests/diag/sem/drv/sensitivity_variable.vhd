-- yavs-test: std=2008
-- expect: E1360 10:13 `v` in a `wait on` list must be a signal
-- LRM 11.3
entity e is end entity;
architecture a of e is
begin
  process
    variable v : bit;
  begin
    wait on v;
  end process;
end architecture;
