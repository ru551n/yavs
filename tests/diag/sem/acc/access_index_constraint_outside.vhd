-- yavs-test: std=2008
-- expect: E1215 7:24 the range is not inside index subtype `POSITIVE`
-- LRM 5.3.2.2, 6.3: the bound of the constraint must belong to the index subtype of the designated array
package p is
  type mem_t is array (positive range <>) of bit;
  type mem_ptr is access mem_t;
  subtype s is mem_ptr(0 to 3);
end package;
