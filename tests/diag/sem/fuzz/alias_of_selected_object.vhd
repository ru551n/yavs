-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.2
package p is
  type bt is record
    a : bit;
    b : bit;
  end record;
  signal s : bt;
  alias al : bit is s.a;
end package;
