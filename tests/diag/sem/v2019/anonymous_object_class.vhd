-- yavs-test: std=2019
-- expect: E1503 8:43 `REAL` is not an integer type
-- LRM 2019 6.5.3: the actual of an object of anonymous type belongs to the type's class
package g is
  generic (c : type is range <>);
  subtype s is c'subtype;
end package;
package i is new work.g generic map (c => 1.5);
