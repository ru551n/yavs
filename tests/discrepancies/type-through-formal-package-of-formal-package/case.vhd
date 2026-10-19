-- LRM 6.5.7.2, 4.8: a type reached through a formal package of a formal package (s.r.arr)
-- as the type of a subprogram parameter.
package ring_g is
  generic (type elem);
  type arr is array (natural range <>) of elem;
end package;
package stats_g is
  generic (package r is new work.ring_g generic map (<>));
end package;
package user_g is
  generic (package s is new work.stats_g generic map (<>));
  function first (x : s.r.arr) return s.r.elem;
end package;
package body user_g is
  function first (x : s.r.arr) return s.r.elem is begin return x(x'left); end function;
end package body;
package int_ring is new work.ring_g generic map (elem => integer);
package int_stats is new work.stats_g generic map (r => work.int_ring);
package int_user is new work.user_g generic map (s => work.int_stats);
entity top is end entity;
architecture a of top is
begin
  process
    constant v : work.int_ring.arr := (4, 5, 6);
  begin
    report "RES " & integer'image(work.int_user.first(v));
    wait;
  end process;
end architecture;
