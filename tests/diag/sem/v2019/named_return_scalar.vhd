-- yavs-test: std=2019
-- expect: E1233 6:21 a named return value needs an array or record type, not `INTEGER`
-- help: drop the return identifier
-- LRM 2019 4.2.1 (lrm-checks L71): `r` takes the target's bounds, a scalar has none
package p is
  function f return r of integer;
end package;
