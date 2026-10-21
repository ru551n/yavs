-- yavs-test: std=2008
-- expect: E1326 10:11 a non-static choice must be the only choice of the only association
-- LRM 9.3.3.3, 9.4.2: 'left of an object whose bounds depend on a generic is only globally static (sim1 and sim2 reject too)
entity e is
  generic (n : positive := 4);
end entity;
architecture a of e is
  signal buf : bit_vector(n - 1 downto 0);
begin
  buf <= (buf'left => '0', others => '1');
end architecture;
