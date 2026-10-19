-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.2, 4.8: a call inside the generic unit is legal, also from a generic package that holds an instance
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
  function again (x : s.r.arr) return integer;
end package;
package body report_g is
  function show (x : s.r.arr) return integer is
  begin
    return s.twice(x);
  end function;
  function again (x : s.r.arr) return integer is
  begin
    return show(x) + 1;
  end function;
end package body;

-- the instance sits inside a generic package; its call is inside that generic
package w_g is
  generic (package s is new work.stats_g generic map (<>));
  function go (x : s.r.arr) return integer;
end package;
package body w_g is
  package rep is new work.report_g generic map (s => s);
  function go (x : s.r.arr) return integer is
  begin
    return rep.show(x) + rep.again(x);
  end function;
end package body;

package int_ring is new work.ring_g generic map (elem => integer);
package int_stats is new work.stats_g generic map (r => work.int_ring);
package int_report is new work.report_g generic map (s => work.int_stats);
package int_w is new work.w_g generic map (s => work.int_stats);

entity t is end entity;
architecture a of t is
  signal n : integer := 0;
begin
end architecture;
