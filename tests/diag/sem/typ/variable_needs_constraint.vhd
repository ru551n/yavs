-- yavs-test: std=2008
-- expect: E1212 8:14 variable `v` needs a fully constrained subtype
-- LRM 6.4.2.4
entity e is end entity;
architecture a of e is
begin
  process
    variable v : string;
  begin
    wait;
  end process;
end architecture;
