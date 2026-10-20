-- yavs-test: std=2019
-- expect: E1410 8:25 range record constant without a value known at analysis
-- LRM 5.3.2.2 (2019): yavs needs the constant's value to build the range (ponytail)
entity e is generic (n : integer := 3); end entity;
architecture a of e is
  signal s : bit_vector(n downto 0);
  constant r : s'range'record := s'range'value;
  signal x : bit_vector(r);
begin
end architecture;
