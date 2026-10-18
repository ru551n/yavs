-- yavs-test: std=2008
-- expect: none
-- LRM 9.4.2: 'left of an object of a locally static subtype is locally static, so it may sit next to others (9.3.3.3)
entity e is end entity;
architecture a of e is
  signal buf : bit_vector(9 downto 0);
begin
  buf <= (buf'left => '0', others => '1');
end architecture;
