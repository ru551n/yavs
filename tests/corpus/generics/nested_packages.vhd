-- difftest: top=nested_packages std=2008 ref=hand ignore=order
-- ignore=order: same-cycle reports of different processes (LRM leaves the order open).
-- ref=hand: the reference simulator crashes on every package declared in a declarative
-- part and does not parse them in processes, subprograms and generates. The second
-- reference simulator agrees with the expected output except for the generate, which it
-- does not parse; the third does not support packages in declarative parts.
-- Packages declared in declarative parts (LRM 4.7, 2008): in a package (body in its
-- body), a package body, an entity, an architecture, a block, a generate (one per
-- iteration), a process and a subprogram; expanded names, use clauses, package signals,
-- protected state in a nested body, a package nested in a nested package.
package outer is
  package inner is
    constant base : integer := 10;
    function scaled(x : integer) return integer;
  end package;
  function via_inner(x : integer) return integer;
end package;

package body outer is
  constant offset : integer := 1;
  package body inner is
    function scaled(x : integer) return integer is
    begin
      return base * x + offset;
    end function;
  end package body;
  package hidden is
    package deep is
      constant d : integer := 5;
    end package;
    function twice(x : integer) return integer;
  end package;
  package body hidden is
    function twice(x : integer) return integer is
    begin
      return 2 * x + deep.d;
    end function;
  end package body;
  function via_inner(x : integer) return integer is
  begin
    return inner.scaled(x) + hidden.twice(x);
  end function;
end package body;

entity nested_packages is
  package ent_pkg is
    constant e : integer := 3;
  end package;
end entity;

architecture a of nested_packages is
  package counters is
    type counter_t is protected
      procedure bump;
      impure function get return integer;
    end protected;
    signal s : integer := 0;
    procedure bump;
    impure function get return integer;
  end package;
  package body counters is
    type counter_t is protected body
      variable n : integer := 0;
      procedure bump is
      begin
        n := n + 1;
      end procedure;
      impure function get return integer is
      begin
        return n;
      end function;
    end protected body;
    shared variable cnt : counter_t;
    procedure bump is
    begin
      cnt.bump;
    end procedure;
    impure function get return integer is
    begin
      return cnt.get;
    end function;
  end package body;
  use work.outer.all;
  use counters.get;
begin
  b : block
    package blk is
      constant k : integer := 7;
      signal t : integer := 1;
    end package;
  begin
    process
    begin
      blk.t <= blk.t + blk.k;
      wait for 1 ns;
      report "blk.t=" & integer'image(blk.t);
      assert blk.t = 8 severity failure;
      wait;
    end process;
  end block;

  g : for i in 1 to 2 generate
    package per is
      constant v : integer := i * 100;
      signal t : integer := i;
    end package;
  begin
    process
    begin
      per.t <= per.t + per.v;
      wait for 1 ns;
      report "g" & integer'image(i) & " t=" & integer'image(per.t);
      assert per.t = 101 * i severity failure;
      wait;
    end process;
  end generate;

  driver : process
  begin
    counters.s <= 42;
    counters.bump;
    wait for 1 ns;
    counters.bump;
    wait;
  end process;

  main : process
    package loc is
      constant c : integer := 9;
      function plus(x : integer) return integer;
    end package;
    package body loc is
      function plus(x : integer) return integer is
      begin
        return x + c;
      end function;
    end package body;
    use loc.all;
    function in_sub(x : integer) return integer is
      package sp is
        constant m : integer := 4;
      end package;
    begin
      return x * sp.m;
    end function;
  begin
    report "inner.base=" & integer'image(inner.base);
    report "via_inner(2)=" & integer'image(via_inner(2));
    report "work.outer.inner.scaled(3)=" & integer'image(work.outer.inner.scaled(3));
    report "ent_pkg.e=" & integer'image(ent_pkg.e);
    report "plus(1)=" & integer'image(plus(1));
    report "in_sub(5)=" & integer'image(in_sub(5));
    assert via_inner(2) = 30 and plus(1) = 10 and in_sub(5) = 20 severity failure;
    wait for 2 ns;
    report "s=" & integer'image(counters.s) & " cnt=" & integer'image(get);
    assert counters.s = 42 and get = 2 severity failure;
    wait;
  end process;
end architecture;
