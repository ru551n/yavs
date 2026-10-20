-- yavs-test: std=2019
-- expect: none
-- LRM 5.3.2.2 (2019): a constant of a range record type is a range
entity e is end entity;
architecture a of e is
  signal s : bit_vector(7 downto 0);
  constant r : s'range'record := s'range'value;
  constant r2 : s'range'record := (direction => ascending, left => 1, right => 4);
  signal x : bit_vector(r);
  signal y : bit_vector(r2);
begin
end architecture;
