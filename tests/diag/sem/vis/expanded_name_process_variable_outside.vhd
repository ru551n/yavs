-- yavs-test: std=2008
-- expect: E1103 14:11 `v` exists but is not visible here
-- help: `v` is declared in process `p1` and is only visible inside it
-- LRM 8.3
entity e is end entity;
architecture a of e is
  signal s : integer;
begin
  p1 : process
    variable v : integer := 0;
  begin
    wait;
  end process;
  s <= p1.v;
end architecture;
