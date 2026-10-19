-- yavs-test: std=2019
-- expect: E1503 8:44 `bit_vector` is unconstrained, the generic array type has an index constraint
-- LRM 2019 6.5.3: the actual of a generic array type with an index constraint is constrained
package g is
  generic (type ct is array (0 to 3) of type is private);
  subtype s is ct;
end package;
package i is new work.g generic map (ct => bit_vector);
