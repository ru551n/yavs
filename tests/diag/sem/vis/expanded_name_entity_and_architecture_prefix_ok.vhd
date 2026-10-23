-- yavs-test: std=2008
-- expect: none
-- LRM 8.3 (inside an entity and its architectures the entity name selects the entity's
-- declarations, the architecture name its own; #639)
entity e is
  port (p, q : in bit);
  type prim is (red, green);
end entity;
architecture a of e is
  signal sg : bit;
  type local is (x, y);
begin
  t : process (e.p, e.q) begin null; end process;
  u : process
    variable w : e.prim := red;
    variable l : a.local := y;
    variable m : work.e.prim := green;
  begin
    a.sg <= e.p and e.q;
    assert a.sg = sg;
    wait;
  end process;
end architecture;
architecture b of e is
begin
  process (e.p) begin null; end process;
end architecture;
