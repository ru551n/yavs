-- yavs-test: std=2019
-- expect: E1233 8:28 a named return value needs a scalar, array or record type, not `PTR`
-- help: drop the return identifier
-- LRM 2019 4.2.1 (lrm-checks L77): `r` takes the target's subtype; an access type has no
-- bounds or range to take
package p is
  type ptr is access integer;
  impure function f return r of ptr;
end package;
