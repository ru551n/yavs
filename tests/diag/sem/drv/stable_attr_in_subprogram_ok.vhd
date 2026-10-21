-- yavs-test: std=2008
-- expect: none
-- LRM 4.2.2.2 forbids these attributes only of formal signal parameters (sim2, sim3 accept)
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
