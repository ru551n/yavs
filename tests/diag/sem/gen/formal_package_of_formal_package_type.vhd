-- yavs-test: std=2008
-- expect: E1300 46:50 type of actual does not match formal `x` type `arr`
-- LRM 6.5.7.2, 4.8
package ring_g is
  generic (type elem);
  type arr is array (natural range <>) of elem;
  function total (x : arr) return integer;
end package;
package body ring_g is
  function total (x : arr) return integer is
  begin
    return x'length;
  end function;
end package body;

package stats_g is
  generic (package r is new work.ring_g generic map (<>));
  function twice (x : r.arr) return integer;
end package;
package body stats_g is
  function twice (x : r.arr) return integer is
  begin
    return r.total(x);
  end function;
end package body;

package report_g is
  generic (package s is new work.stats_g generic map (<>));
  function show (x : s.r.arr) return integer;
end package;
package body report_g is
  function show (x : s.r.arr) return integer is
  begin
    return s.twice(x);
  end function;
end package body;

package int_ring is new work.ring_g generic map (elem => integer);
package int_stats is new work.stats_g generic map (r => work.int_ring);
package int_report is new work.report_g generic map (s => work.int_stats);

entity t is end entity;
architecture a of t is
  constant v : work.int_ring.arr := (1, 2, 3);
  constant one : integer := work.int_stats.twice(v);
  constant bad : integer := work.int_report.show(v);
begin
end architecture;
