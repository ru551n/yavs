-- difftest: top=impure_const_once std=2008
-- A constant whose initial value calls an impure function with side effects on a shared
-- variable is elaborated exactly once (LRM 14.4.2, 14.5): in a package, an architecture, a
-- process and a subprogram declarative part (#251).
package impure_const_once_p is
  type rec_t is record id : natural; end record;
  type pt is protected
    impure function create(name : string := ""; size : positive := positive'high) return rec_t;
  end protected;
  impure function new_thing(name : string := ""; size : positive := positive'high) return rec_t;
end package;
package body impure_const_once_p is
  type pt is protected body
    variable n : natural := 0;
    impure function create(name : string := ""; size : positive := positive'high) return rec_t is
    begin
      n := n + 1;
      return (id => n);
    end;
  end protected body;
  shared variable messenger : pt;
  impure function new_thing(name : string := ""; size : positive := positive'high) return rec_t is
  begin
    return messenger.create(name, size);
  end;
end package body;
use work.impure_const_once_p.all;
package impure_const_once_q is
  constant k : rec_t := new_thing("pkg");
end package;
use work.impure_const_once_p.all;
use work.impure_const_once_q.all;
entity impure_const_once is end;
architecture a of impure_const_once is
  constant ca : rec_t := new_thing("arch");
  impure function sub return natural is
    constant cs : rec_t := new_thing("sub");
  begin
    return cs.id;
  end;
begin
  process
    constant cp : rec_t := new_thing("main");
    variable s1, s2, nx : natural;
  begin
    s1 := sub;
    s2 := sub;
    nx := new_thing("x").id;
    report "k=" & integer'image(k.id) & " ca=" & integer'image(ca.id) & " cp="
      & integer'image(cp.id) & " s1=" & integer'image(s1) & " s2=" & integer'image(s2)
      & " next=" & integer'image(nx);
    assert k.id = 1 and ca.id = 2 and cp.id = 3 and s1 = 4 and s2 = 5 and nx = 6
      severity failure;
    wait;
  end process;
end;
