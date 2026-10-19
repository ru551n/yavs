-- (from the differential corpus)
-- ref=hand: the reference simulators reject package instantiations in process,
-- subprogram and generate declarative parts (LRM 2008 allows them: 4.3, 11.3,
-- 11.8) and instances of a generic subprogram of a package instance; the
-- third reference simulator runs it but gives a wrong `proc=` value (an `out`
-- parameter of a procedure with a local package instance).
package counter_g is
  generic (start : integer; step : integer := 1);
  function next_of(x : integer) return integer;
  constant first : integer := start + step;
end package;
package body counter_g is
  function next_of(x : integer) return integer is
  begin
    return x + step;
  end function;
end package body;

package acc_g is
  generic (base : integer);
  type acc_t is protected
    procedure add(x : integer);
    impure function get return integer;
  end protected;
end package;
package body acc_g is
  type acc_t is protected body
    variable total : integer := base;
    procedure add(x : integer) is
    begin
      total := total + x;
    end procedure;
    impure function get return integer is
    begin
      return total;
    end function;
  end protected body;
end package body;

package sub_g is
  generic (offset : integer);
  function shift
    generic (type t; function to_int(x : t) return integer)
    parameter (x : t) return integer;
end package;
package body sub_g is
  function shift
    generic (type t; function to_int(x : t) return integer)
    parameter (x : t) return integer is
  begin
    return to_int(x) + offset;
  end function;
end package body;
package sub100 is new work.sub_g generic map (offset => 100);

package conv is
  function bool_int(b : boolean) return integer;
end package;
package body conv is
  function bool_int(b : boolean) return integer is
  begin
    if b then return 1; else return 0; end if;
  end function;
end package body;

use work.conv.all;
entity pkg_instance_local is end entity;
architecture t of pkg_instance_local is
  function shift_bool is new work.sub100.shift generic map (t => boolean, to_int => bool_int);

  function in_func(x : integer) return integer is
    package local is new work.counter_g generic map (start => x, step => 3);
  begin
    return local.next_of(local.first);
  end function;

  procedure in_proc(x : integer; r : out integer) is
    package local is new work.counter_g generic map (start => 2 * x);
  begin
    r := local.first;
  end procedure;

  -- A fresh accumulator on every call.
  impure function fresh(k : integer) return integer is
    package a is new work.acc_g generic map (base => k);
    variable v : a.acc_t;
  begin
    v.add(k);
    v.add(1);
    return v.get;
  end function;
begin
  g : for i in 1 to 2 generate
    package in_gen is new work.counter_g generic map (start => i * 1000, step => i);
  begin
    process
    begin
      wait for 2 ns * i;
      report "gen " & integer'image(i) & " first=" & integer'image(in_gen.first)
        & " next=" & integer'image(in_gen.next_of(0));
      wait;
    end process;
  end generate;

  ig : if true generate
    package in_if is new work.counter_g generic map (start => -1);
  begin
    process
    begin
      wait for 5 ns;
      report "if first=" & integer'image(in_if.first);
      wait;
    end process;
  end generate;

  process
    package in_process is new work.counter_g generic map (start => 30, step => 3);
    package acc is new work.acc_g generic map (base => 500);
    variable a : acc.acc_t;
    variable r : integer;
  begin
    wait for 10 ns;
    report "process first=" & integer'image(in_process.first)
      & " next=" & integer'image(in_process.next_of(1));
    a.add(5);
    report "process acc=" & integer'image(a.get);
    report "func=" & integer'image(in_func(4)) & " " & integer'image(in_func(10));
    in_proc(6, r);
    report "proc=" & integer'image(r);
    in_proc(7, r);
    report "proc=" & integer'image(r);
    report "fresh=" & integer'image(fresh(3)) & " " & integer'image(fresh(3)) & " " & integer'image(fresh(8));
    report "shift=" & integer'image(shift_bool(true)) & " " & integer'image(shift_bool(false));
    wait;
  end process;
end architecture;
