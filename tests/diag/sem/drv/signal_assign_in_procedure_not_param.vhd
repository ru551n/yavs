-- yavs-test: std=2008
-- expect: E1810 9:5 signal `s` is driven from a subprogram and is not a parameter
-- LRM 11.3
entity e is end entity;
architecture a of e is
  signal s : bit;
  procedure pr is
  begin
    s <= '1';
  end procedure;
begin
end architecture;
