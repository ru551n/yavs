-- yavs-test: std=2008
-- expect: none
-- LRM 5.4, 8.1, 9.3.7
package p is
  type int_ptr is access integer;
  type str_ptr is access string;
  type rec_t is record
    a : integer;
    s : str_ptr;
  end record;
  type rec_ptr is access rec_t;
  type ptr_ptr is access int_ptr;
  procedure run;
end package;

package body p is
  procedure run is
    variable i  : int_ptr;
    variable s  : str_ptr;
    variable r  : rec_ptr;
    variable pp : ptr_ptr;
    variable n  : integer;
  begin
    i := new integer;
    i := new integer'(5);
    i.all := i.all + 1;
    s := new string(1 to 5);
    s := new string'("hello");
    s(1) := 'H';
    s.all(2) := 'E';
    n := s'length + s.all'length;
    for k in s'range loop
      s(k) := 'x';
    end loop;
    s(1 to 2) := "ab";
    r := new rec_t'(a => 1, s => null);
    r.a := 2;
    r.all.s := s;
    r.s(1) := 'y';
    pp := new int_ptr'(i);
    pp.all.all := 3;
    if i /= null and s = null then
      i := null;
    end if;
  end procedure;
end package body;
