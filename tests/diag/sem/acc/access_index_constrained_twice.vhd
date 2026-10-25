-- yavs-test: std=2008
-- expect: E1210 7:18 the index ranges of this array subtype are already constrained
-- LRM 5.3.2.2, 6.3: the constraint applies to the designated subtype, already constrained by the parent
package p is
  type mem_ptr is access bit_vector;
  subtype s is mem_ptr(0 to 3);
  subtype t is s(1 to 2);
end package;
