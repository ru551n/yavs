-- yavs-test: std=2008
-- expect: E1002 7:31 entity `ghost` not found in library `work`
-- LRM 7.3.2
entity e is end entity;
architecture a of e is
  component c is port (x : in bit); end component;
  for all : c use entity work.ghost;
begin
end architecture;
