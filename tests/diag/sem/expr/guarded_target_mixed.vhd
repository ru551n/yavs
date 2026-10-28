-- yavs-test: std=2008
-- expect: E1360 15:3 the target mixes guarded and unguarded signals
-- expect: E1360 16:3 a guarded signal is the target of an unguarded concurrent signal assignment
-- LRM 11.6: a concurrent signal assignment target is all guarded signals, then in a guarded assignment, or none (#812)
entity e is
end entity;
architecture a of e is
  function res (v : bit_vector) return bit is
  begin
    return v(v'left);
  end function;
  signal i, j : res bit bus;
  signal k, l : bit;
begin
  (i, k) <= transport bit_vector'("10");
  i <= transport k;
  b : block (k = '1')
  begin
    (i, j) <= guarded bit_vector'("10");
    l <= k;
  end block;
end architecture;
