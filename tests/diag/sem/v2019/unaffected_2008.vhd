-- yavs-test: std=2008
-- expect: E0004 11:10 `unaffected` as an assignment or return value requires VHDL-2019
-- LRM 2019 10.6.2.1
entity e is
end entity;
architecture a of e is
begin
  process
    variable v : integer;
  begin
    v := unaffected;
    wait;
  end process;
end architecture;
