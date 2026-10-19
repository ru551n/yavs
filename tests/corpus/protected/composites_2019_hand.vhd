-- difftest: top=composites_2019_hand std=2019 ref=hand
-- 2019 composites of protected types and access types to them (LCS-2016-014, 014a, #361):
-- every element of an array, record or allocated object is its own initialised protected
-- object; method calls through indexing, selection and `.all`; element and whole-array
-- variable parameters; arrays in protected state, subprogram frames and shared variables;
-- deallocation. ref=hand: of the four reference simulators only one runs it (the others
-- reject arrays of protected types), and it crashes on the composite state of `holder_t`;
-- the .expected is its output without `holder_t`, plus the `h` line by hand (xs(0) once,
-- xs(2) twice, r.c(0) three times: 6).
package composites_2019_hand_pkg is
  type counter_t is protected
    procedure inc;
    impure function get return integer;
  end protected;
  type counters_t is array (natural range <>) of counter_t;
  type grid_t is array (0 to 1, 0 to 1) of counter_t;
  type pair_t is record a : counter_t; c : counters_t(0 to 1); end record;
  type pairs_t is array (0 to 1) of pair_t;
  type pairs_ptr is access pairs_t;
  type counters_ptr is access counters_t;
  type holder_t is protected
    procedure bump (i : natural);
    impure function sum return integer;
  end protected;
end package;

package body composites_2019_hand_pkg is
  type counter_t is protected body
    variable n : integer := 0;
    procedure inc is begin n := n + 1; end procedure;
    impure function get return integer is begin return n; end function;
  end protected body;
  type holder_t is protected body
    variable xs : counters_t(0 to 2);
    variable r : pair_t;
    procedure bump (i : natural) is begin xs(i).inc; r.c(i mod 2).inc; end procedure;
    impure function sum return integer is
      variable s : integer := 0;
    begin
      for i in xs'range loop
        s := s + xs(i).get;
      end loop;
      return s + r.c(0).get + r.c(1).get;
    end function;
  end protected body;
end package body;

use work.composites_2019_hand_pkg.all;
entity composites_2019_hand is end entity;
architecture t of composites_2019_hand is
  shared variable h : holder_t;
  shared variable sv : counters_t(0 to 2);
  procedure twice (variable x : inout counter_t) is begin x.inc; x.inc; end procedure;
  impure function local_sum return integer is
    variable l : counters_t(1 to 3);
  begin
    l(2).inc;
    return l(1).get + l(2).get + l(3).get;
  end function;
begin
  p1 : process begin sv(1).inc; wait; end process;
  p2 : process begin sv(1).inc; sv(2).inc; wait; end process;
  main : process
    variable u : counters_t(0 to 2);
    variable m : grid_t;
    variable x : pairs_t;
    variable xp : pairs_ptr;
    variable up : counters_ptr;
  begin
    twice(u(1)); u(0).inc; u(1).inc; u(2).inc; u(2).inc;
    report "u " & integer'image(u(0).get) & integer'image(u(1).get) & integer'image(u(2).get);
    assert u(1).get = 3 severity failure;
    m(1, 0).inc;
    report "m " & integer'image(m(0, 0).get) & integer'image(m(1, 0).get);
    x(1).a.inc; x(1).c(1).inc; x(1).c(1).inc;
    report "x " & integer'image(x(0).a.get) & integer'image(x(1).a.get) & integer'image(x(1).c(1).get);
    xp := new pairs_t; xp(0).c(0).inc; xp.all(0).c(0).inc;
    report "xp " & integer'image(xp(0).c(0).get) & integer'image(xp(1).c(0).get);
    up := new counters_t(5 to 7); up(6).inc;
    report "up " & integer'image(up'length) & integer'image(up(6).get) & integer'image(up(7).get);
    deallocate(xp); deallocate(up);
    assert xp = null and up = null severity failure;
    h.bump(0); h.bump(2); h.bump(2);
    report "h " & integer'image(h.sum);
    report "f " & integer'image(local_sum) & integer'image(local_sum);
    wait for 1 ns;
    report "sv " & integer'image(sv(0).get) & integer'image(sv(1).get) & integer'image(sv(2).get);
    wait;
  end process;
end architecture;
