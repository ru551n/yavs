-- yavs-test: std=2008
-- LRM 9.3.3.3: near misses. `others` with an element, a range choice with a
-- piece of the aggregate type, and `others` with an array-typed element.
package p is
  type bv_arr is array (0 to 1) of bit_vector(1 downto 0);
  constant a : bit_vector(1 downto 0) := "10";
  constant e : bit_vector(3 downto 0) := (others => '0');
  constant r : bit_vector(3 downto 0) := (1 downto 0 => a, others => '1');
  constant m : bv_arr := (others => a);
end package;
