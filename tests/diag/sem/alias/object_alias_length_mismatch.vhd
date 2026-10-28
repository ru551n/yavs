-- yavs-test: std=2008
-- expect: E1405 8:15 the alias subtype has 22 elements, the aliased object 24
-- LRM 6.6.2: each element of the object has a matching element of the alias subtype (#813)
entity e is end;
architecture a of e is
  signal r : bit_vector(0 to 31);
  alias ok : bit_vector(23 downto 0) is r(8 to 31);
  alias bad : bit_vector(21 downto 0) is r(8 to 31);
begin
end;
