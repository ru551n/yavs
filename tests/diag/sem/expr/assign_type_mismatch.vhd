-- yavs-test: std=2008
-- expect: E1300 11:10 expected `integer`, found `bit`
-- LRM 10.6.2
entity e is end entity;
architecture a of e is
begin
  process
    variable i : integer;
    variable b : bit;
  begin
    i := b;
    wait;
  end process;
end architecture;
