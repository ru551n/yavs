-- yavs-test: std=2008
-- expect: E1360 10:5 `v` is a variable; use `:=` to assign it
-- LRM 10.5
entity e is end entity;
architecture a of e is
begin
  process
    variable v : integer;
  begin
    v <= 1;
    wait;
  end process;
end architecture;
