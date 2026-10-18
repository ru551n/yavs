-- yavs-test: std=2008
-- expect: none
-- LRM 16.2
entity e is end entity;
architecture a of e is
  signal s : bit;
  procedure pr (signal x : in bit) is
    variable b : boolean;
  begin
    b := x'event;
  end procedure;
begin
end architecture;
