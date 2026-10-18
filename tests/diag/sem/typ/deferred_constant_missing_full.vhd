-- yavs-test: std=2008
-- expect: E1230 6:12 deferred constant `d` has no full declaration in the package body
-- LRM 4.8
package p is
  constant c : integer;
  constant d : integer;
end package;
package body p is
  constant c : integer := 1;
end package body;
