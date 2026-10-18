-- yavs-test: std=2008
-- expect: E1610 8:16 the aliased name is not a static name of an object
-- LRM 6.6.2, 8.1
package p is
  signal runner : bit_vector(0 to 15);
  signal idx : natural;
  alias ok_part is runner(1);
  alias bad is runner(idx);
end package;
