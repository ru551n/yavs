-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.3.3
-- Nested named rows: the choices of dimension k are of the k-th index type (#457).
package p is
  type remains is (r0, r1, r2);
  type branch is array (remains, bit) of remains;
  constant br_table : branch := (r0 => ('0' => r0, '1' => r1),
                                 r1 => ('0' => r2, '1' => r1),
                                 r2 => ('0' => r2, '1' => r0));
end package;

-- Elements that are arrays themselves: their choices are of the element's index type.
package p2 is
  type bv is array (natural range 0 to 1) of bit;
  type cell is array (bit, boolean) of bit;
  type remains is (r0, r1);
  type m is array (remains, bit) of bv;
  type m3 is array (remains, bit) of cell;
  constant a : m := (r0 => ('0' => (0 => '1', 1 => '0'), '1' => "01"), r1 => (others => (others => '1')));
  constant b : m3 := (r0 => ('0' => (('0' => (false => '1', true => '0'), '1' => (false => '0', true => '1'))), others => (others => (others => '1'))), others => (others => (others => (others => '0'))));
end package;
