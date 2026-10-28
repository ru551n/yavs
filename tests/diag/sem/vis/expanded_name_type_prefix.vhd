-- yavs-test: std=2008
-- expect: E1104 5:24 `BOOLEAN` is not a region an expanded name can select from
-- LRM 8.3: the prefix of an expanded name denotes a declarative region (#822)
package p is
  constant t : time := boolean.foo;
end package;
