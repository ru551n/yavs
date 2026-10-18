-- yavs-test: std=2008
-- expect: E1223 8:12 a constant cannot be of file type `ft`
-- LRM 6.4.2.2
entity e is
end entity;
architecture a of e is
  type ft is file of integer;
  constant c : ft;
begin
end architecture;
