-- difftest: top=package_const_once std=2008
-- Constants are evaluated once, at elaboration (#73): impure initialisers count their calls
-- (reports at elaboration would print source paths in the reference output), and every
-- later use (two processes, several reads, a subprogram) sees the one value. Covers package
-- constants whose bounds are known only at run time and composite architecture constants.
package pco_count is
  type rec_t is record
    a, b : natural;
  end record;
  impure function next_n return natural;
  impure function mk(tag : string) return string;
  impure function mkr return rec_t;
end package;
package body pco_count is
  type counter_t is protected
    impure function next_id return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is begin n := n + 1; return n; end;
  end protected body;
  shared variable cnt : counter_t;
  impure function next_n return natural is begin return cnt.next_id; end;
  impure function mk(tag : string) return string is
    variable n : natural;
  begin
    n := next_n;
    return tag & integer'image(n);
  end;
  impure function mkr return rec_t is
    variable r : rec_t;
  begin
    r.a := next_n;
    r.b := next_n;
    return r;
  end;
end package body;

use work.pco_count.all;
package pco_consts is
  constant S : string := mk("pkg");
  constant N : natural := S'length;
  impure function get_s return string;
end package;
package body pco_consts is
  constant SB : string := mk("body");
  impure function get_s return string is begin return S & SB; end;
end package body;

use work.pco_count.all;
use work.pco_consts.all;
entity package_const_once is end;
architecture t of package_const_once is
  constant A : string := mk("arch");
  constant R : rec_t := mkr;
begin
  p1 : process begin
    report "p1 " & S & " " & S & " " & get_s & " " & integer'image(N);
    report "p1 " & A & " " & A & " " & integer'image(R.a) & " " & integer'image(R.b);
    assert S = S and A = A severity failure;
    wait;
  end process;
  p2 : process begin
    wait for 1 ns;
    report "p2 " & S & " " & get_s & " " & A & " " & integer'image(R.a + R.b);
    report "calls so far " & integer'image(next_n - 1);
    wait;
  end process;
end;
