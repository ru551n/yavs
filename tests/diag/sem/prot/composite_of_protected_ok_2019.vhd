-- yavs-test: std=2019
-- expect: none
-- LCS-2016-014, 014a: arrays, records and access types of protected types; method calls
-- through indexing, selection and `.all`; shared variables whose scalar subelements are all
-- protected; a record may mix protected and other elements (permissive, lrm-checks)
package p is
  type pt is protected
    procedure inc;
    impure function get return integer;
  end protected;
  type pa is array (natural range <>) of pt;
  type rec is record a : pt; c : pa(0 to 1); end record;
  type rec_ptr is access rec;
  type pa_ptr is access pa;
end package;
package body p is
  type pt is protected body
    variable n : integer := 0;
    procedure inc is begin n := n + 1; end procedure;
    impure function get return integer is begin return n; end function;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  shared variable sv : pa(0 to 3);
  procedure bump (variable x : inout pa) is
  begin
    x(x'left).inc;
  end procedure;
begin
  process
    variable v : pa(1 to 2);
    variable r : rec;
    variable rp : rec_ptr;
    variable ap : pa_ptr;
  begin
    v(1).inc;
    bump(v);
    r.a.inc;
    r.c(1).inc;
    r.a.inc;
    rp := new rec;
    rp.all.c(0).inc;
    rp.a.inc;
    ap := new pa(0 to 4);
    ap(3).inc;
    deallocate(rp);
    deallocate(ap);
    wait;
  end process;
end architecture;
