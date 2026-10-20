-- difftest: top=read_failed std=2008 exit=1
-- A textio READ that fails without a GOOD argument reports an error and the run goes on; the
-- line is left as it was (the variable's value after a failure is not specified).
use std.textio.all;
entity read_failed is end;
architecture a of read_failed is
begin
  process
    variable l : line;
    variable i : integer;
    variable r : real;
    variable b : bit;
    variable bo : boolean;
    variable c : character;
    variable t : time;
    variable s : string(1 to 3);
    variable bv : bit_vector(1 to 3);
  begin
    l := new string'("zzz");
    read(l, i);
    assert l.all = "zzz" report "line changed" severity failure;
    l := new string'("zzz"); read(l, r);
    l := new string'("zzz"); read(l, b);
    l := new string'("zzz"); read(l, bo);
    l := new string'(""); read(l, c);
    l := new string'("zzz"); read(l, t);
    l := new string'("zz"); read(l, s);
    l := new string'("zzz"); read(l, bv);
    l := new string'("zzz"); hread(l, bv);
    l := new string'("zzz"); oread(l, bv);
    report "end";
    wait;
  end process;
end;
