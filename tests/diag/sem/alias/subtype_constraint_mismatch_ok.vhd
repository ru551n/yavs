-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.2
package p is
  signal a : bit_vector(0 to 3);
  alias b : bit_vector(3 downto 0) is a;
end package;
