-- difftest: top=c std=2008
-- Access values allocated by functions that the initial values of a shared protected
-- object's variables call, at any depth (#246).
use std.textio.all;
entity c is end;
architecture t of c is
  type mb_t is record size : natural; end record;
  type mb_ptr_t is access mb_t;
  type pair_t is record a, b : mb_ptr_t; end record;
  type ptr_array_t is array (natural range <>) of mb_ptr_t;
  type ptr_array_ptr_t is access ptr_array_t;
  impure function create(size : natural) return mb_ptr_t is
  begin
    return new mb_t'(size => size);
  end function;
  impure function wrap(size : natural) return mb_ptr_t is
    variable r : mb_ptr_t;
  begin
    r := create(size);
    r.size := r.size + 1;
    return r;
  end function;
  impure function make_pair return pair_t is
  begin
    return (a => wrap(10), b => create(20));
  end function;
  impure function make_array(n : natural) return ptr_array_ptr_t is
    variable r : ptr_array_ptr_t := new ptr_array_t(0 to n - 1);
  begin
    for i in r'range loop
      r(i) := wrap(100 + i);
    end loop;
    return r;
  end function;
  impure function make_line return line is
    variable l : line;
  begin
    write(l, string'("hello"));
    write(l, 42);
    return l;
  end function;
  type pt is protected
    impure function sum return natural;
    impure function text return string;
  end protected;
  type pt is protected body
    variable mb : mb_ptr_t := create(5);
    variable nested : mb_ptr_t := wrap(6);
    variable pair : pair_t := make_pair;
    variable arr : ptr_array_ptr_t := make_array(3);
    variable l : line := make_line;
    impure function init_items return ptr_array_ptr_t is
    begin
      return make_array(2);
    end;
    variable items : ptr_array_ptr_t := init_items;
    impure function sum return natural is
      variable s : natural := mb.size + nested.size + pair.a.size + pair.b.size;
    begin
      for i in arr'range loop
        s := s + arr(i).size;
      end loop;
      for i in items'range loop
        s := s + items(i).size;
      end loop;
      return s;
    end;
    impure function text return string is begin return l.all; end;
  end protected body;
  shared variable p : pt;
begin
  process
    variable v : mb_ptr_t := wrap(1);
  begin
    -- 5 + 7 + 11 + 20 + (101 + 102 + 103) + (101 + 102)
    report "sum=" & integer'image(p.sum);
    assert p.sum = 552 severity failure;
    report "text=" & p.text;
    assert p.text = "hello42" severity failure;
    report "v=" & integer'image(v.size);
    wait;
  end process;
end;
