-- difftest: top=local_generic_packages_hand std=2008 ref=hand ignore=order
-- ignore=order: same-cycle reports of different processes (LRM leaves the order open).
-- ref=hand: the reference simulator instantiates no generic package declared in a
-- declarative part that has a body ("cannot be instantiated until its body has been
-- analysed"), crashes on signals of an instance of one declared in a library package and
-- does not parse packages in processes, subprograms and generates; the second reference
-- simulator fails with an internal error and the third does not support them. The values
-- below follow from the generic maps.
-- Generic packages declared in declarative parts with bodies (2008 local generic
-- packages): in a library package (body in its body), a package body, an architecture, a
-- generic package (instantiated in its body and through an instance of it), a generate
-- (one instance per iteration), a process and a subprogram; functions, signals and
-- protected state per instance.
package lib_pkg is
  package scale_g is
    generic (k : integer);
    function scale(x : integer) return integer;
    signal s : integer := k;
  end package;
  package by3 is new scale_g generic map (k => 3);
  function via_body(x : integer) return integer;
end package;

package body lib_pkg is
  package body scale_g is
    function scale(x : integer) return integer is
    begin
      return k * x;
    end function;
  end package body;
  package add_g is
    generic (a : integer);
    function add(x : integer) return integer;
  end package;
  package add5 is new add_g generic map (a => 5);
  package body add_g is
    function add(x : integer) return integer is
    begin
      return x + a;
    end function;
  end package body;
  package by7 is new scale_g generic map (k => 7);
  function via_body(x : integer) return integer is
  begin
    return add5.add(by7.scale(x)) + by3.scale(x);
  end function;
end package body;

-- A generic package nested in a generic package, its body in the outer body, using both
-- generics; instantiated in the outer body and from outside through an outer instance.
package level1_g is
  generic (k : integer);
  package level2_g is
    generic (m : integer);
    function prod return integer;
  end package;
  function via_fixed return integer;
end package;

package body level1_g is
  package body level2_g is
    function prod return integer is
    begin
      return k * m;
    end function;
  end package body;
  package fixed is new level2_g generic map (m => 3);
  package hidden is new level2_g generic map (m => 5);
  function via_fixed return integer is
  begin
    return fixed.prod + hidden.prod;
  end function;
end package body;

package l1_2 is new work.level1_g generic map (k => 2);

entity local_generic_packages_hand is
end entity;

architecture a of local_generic_packages_hand is
  package acc_g is
    generic (init : integer);
    type acc_t is protected
      procedure add(x : integer);
      impure function get return integer;
    end protected;
    signal s : integer := init;
    procedure add(x : integer);
    impure function get return integer;
  end package;
  package body acc_g is
    type acc_t is protected body
      variable n : integer := init;
      procedure add(x : integer) is
      begin
        n := n + x;
      end procedure;
      impure function get return integer is
      begin
        return n;
      end function;
    end protected body;
    shared variable acc : acc_t;
    procedure add(x : integer) is
    begin
      acc.add(x);
    end procedure;
    impure function get return integer is
    begin
      return acc.get;
    end function;
  end package body;
  package acc1 is new acc_g generic map (init => 100);
  package acc2 is new acc_g generic map (init => 200);
  package by2 is new work.lib_pkg.scale_g generic map (k => 2);
  package l1_local is new work.level1_g generic map (k => 3);
  package l2 is new l1_local.level2_g generic map (m => 4);
begin
  g : for i in 1 to 2 generate
    package acc_i is new acc_g generic map (init => 10 * i);
  begin
    process
    begin
      acc_i.add(i);
      acc_i.s <= acc_i.get;
      wait for 1 ns;
      report "g" & integer'image(i) & " get=" & integer'image(acc_i.get) & " s="
        & integer'image(acc_i.s);
      assert acc_i.get = 11 * i and acc_i.s = 11 * i severity failure;
      wait;
    end process;
  end generate;

  driver : process
  begin
    acc1.add(1);
    acc2.add(2);
    acc1.s <= 11;
    acc2.s <= 22;
    work.lib_pkg.by3.s <= 33;
    wait;
  end process;

  main : process
    package sq_g is
      generic (d : integer);
      function sq return integer;
    end package;
    package body sq_g is
      function sq return integer is
      begin
        return d * d;
      end function;
    end package body;
    package sq6 is new sq_g generic map (d => 6);
    function cube(x : integer) return integer is
      package mul_g is
        generic (f : integer);
        function mul(y : integer) return integer;
      end package;
      package body mul_g is
        function mul(y : integer) return integer is
        begin
          return f * y;
        end function;
      end package body;
      package by_x is new mul_g generic map (f => x);
    begin
      return by_x.mul(x * x);
    end function;
  begin
    report "via_body=" & integer'image(work.lib_pkg.via_body(2));
    report "by2=" & integer'image(by2.scale(21));
    report "l1_2=" & integer'image(work.l1_2.via_fixed);
    report "l1_local=" & integer'image(l1_local.via_fixed);
    report "l2=" & integer'image(l2.prod);
    report "sq6=" & integer'image(sq6.sq);
    report "cube(3)=" & integer'image(cube(3)) & " cube(4)=" & integer'image(cube(4));
    assert work.lib_pkg.via_body(2) = 25 and by2.scale(21) = 42 severity failure;
    assert work.l1_2.via_fixed = 16 and l1_local.via_fixed = 24 and l2.prod = 12
      severity failure;
    assert sq6.sq = 36 and cube(3) = 27 and cube(4) = 64 severity failure;
    wait for 1 ns;
    report "acc1=" & integer'image(acc1.get) & " s=" & integer'image(acc1.s);
    report "acc2=" & integer'image(acc2.get) & " s=" & integer'image(acc2.s);
    report "by3.s=" & integer'image(work.lib_pkg.by3.s) & " by2.s=" & integer'image(by2.s);
    assert acc1.get = 101 and acc2.get = 202 severity failure;
    assert acc1.s = 11 and acc2.s = 22 and work.lib_pkg.by3.s = 33 and by2.s = 2
      severity failure;
    wait;
  end process;
end architecture;
