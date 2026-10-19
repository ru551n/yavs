-- yavs-test: std=2008
-- expect: none
-- LRM 10.5.2.1, 10.6.2.1 (#350)
-- The type of an aggregate target comes from the right-hand side, so its element
-- associations may name record fields, and may be nested aggregate targets, for variables
-- and signals alike; a matrix target is an aggregate of aggregates.
entity e is end entity;
architecture a of e is
  type rec is record a, b : integer; end record;
  type rec2 is record r : rec; c : integer; end record;
  type row is array (0 to 1) of integer;
  type mat is array (0 to 1) of row;
  type m2 is array (0 to 1, 0 to 1) of integer;
  function f return rec is begin return (1, 2); end function;
  function g return rec2 is begin return ((1, 2), 3); end function;
  function h return mat is begin return ((1, 2), (3, 4)); end function;
  function k return m2 is begin return ((1, 2), (3, 4)); end function;
  signal sx, sy, sz, sw : integer;
begin
  process
    variable x, y, z, p, q : integer;
  begin
    (b => y, a => x) := f;
    (a => x, b => y) := rec'(5, 6);
    ((x, y), z) := g;
    (r => (b => y, a => x), c => z) := g;
    ((x, y), (z, p)) := h;
    ((x, y), (z, p)) := k;
    (sx, sy) <= rec'(5, 6);
    (b => sy, a => sx) <= f;
    ((sx, sy), sz) <= g;
    ((sx, sy), (sz, sw)) <= k;
    wait;
  end process;
end architecture;
