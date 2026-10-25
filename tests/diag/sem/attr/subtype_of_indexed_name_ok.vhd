-- yavs-test: std=2008
-- expect: none
-- LRM 16.2: S'SUBTYPE of an object name, here an indexed name; the parentheses are not a constraint
entity r10 is end entity;
architecture a of r10 is
  type vec is array (natural range <>) of bit_vector(3 downto 0);
  signal arr : vec(0 to 1);
  signal e : arr(0)'subtype;
begin
end architecture;
