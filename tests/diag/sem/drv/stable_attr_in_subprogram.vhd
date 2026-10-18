-- rsim-test: std=2008
-- expect: E1620 10:10 implicit signal attribute `stable` cannot be used in a subprogram
-- LRM 16.2
entity e is end entity;
architecture a of e is
  signal s : bit;
  procedure pr is
    variable b : boolean;
  begin
    b := s'stable;
  end procedure;
begin
end architecture;
