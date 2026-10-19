-- difftest: top=local_generic_packages std=2008 ignore=order
-- ignore=order: same-cycle reports of different processes (LRM leaves the order open).
-- Generic packages declared in declarative parts (2008 local generic packages) without
-- bodies: in a generic package, an entity and an architecture, instantiated in the same
-- region, in a nested one (a block), from another unit and inside an instance of the
-- enclosing generic package; constants and signals per instance. Local generic packages
-- with bodies, in library packages and package bodies, generates, processes and
-- subprograms: local_generic_packages_hand (ref=hand; the reference simulator rejects or
-- crashes on them).
-- A generic package nested in a generic package, using the outer generic.
package outer_g is
  generic (k : integer);
  package inner_g is
    generic (m : integer);
    constant prod : integer := k * m;
  end package;
  package fixed is new inner_g generic map (m => 10);
end package;

package outer2 is new work.outer_g generic map (k => 2);

entity local_generic_packages is
  package ent_g is
    generic (e : integer);
    constant twice : integer := 2 * e;
  end package;
end entity;

architecture a of local_generic_packages is
  package ent5 is new ent_g generic map (e => 5);
  package cnt_g is
    generic (init : integer);
    signal s : integer := init;
    constant c : integer := init + 1;
  end package;
  package c1 is new cnt_g generic map (init => 100);
  package c2 is new cnt_g generic map (init => 200);
  package outer3 is new work.outer_g generic map (k => 3);
  package inner5 is new outer3.inner_g generic map (m => 5);
begin
  c1.s <= c1.c after 1 ns;
  c2.s <= c2.c + 1 after 1 ns;

  b : block
    package c3 is new cnt_g generic map (init => 300);
  begin
    c3.s <= c3.c + 2 after 1 ns;
    process
    begin
      wait for 2 ns;
      report "c3.s=" & integer'image(c3.s) & " c3.c=" & integer'image(c3.c);
      assert c3.s = 303 and c3.c = 301 severity failure;
      wait;
    end process;
  end block;

  main : process
  begin
    report "ent5.twice=" & integer'image(ent5.twice);
    report "outer2.fixed.prod=" & integer'image(work.outer2.fixed.prod);
    report "outer3.fixed.prod=" & integer'image(outer3.fixed.prod);
    report "inner5.prod=" & integer'image(inner5.prod);
    assert ent5.twice = 10 severity failure;
    assert work.outer2.fixed.prod = 20 and inner5.prod = 15 severity failure;
    wait for 2 ns;
    report "c1.s=" & integer'image(c1.s) & " c2.s=" & integer'image(c2.s);
    assert c1.s = 101 and c2.s = 202 severity failure;
    wait;
  end process;
end architecture;
