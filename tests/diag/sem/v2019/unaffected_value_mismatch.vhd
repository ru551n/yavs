-- yavs-test: std=2019
-- expect: E1300 12:10 expected `integer`, found a character literal
-- LRM 2019 10.6.2.1
entity e is
end entity;
architecture a of e is
  signal c : boolean;
begin
  process
    variable v : integer;
  begin
    v := '1' when c else unaffected;
    wait;
  end process;
end architecture;
