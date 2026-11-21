-- difftest: top=state_param_copy std=2008 ref=hand
-- A scalar state variable of a protected type passed to an inout parameter is passed by copy
-- (LRM 4.2.2.2): a subprogram of the body that reads the state by its own name, and a package
-- procedure that calls back into a method of the same object, see the old value until the call
-- returns; a procedure that reaches neither keeps it by reference (#1187). sim1 passes these by
-- reference and sees the new value; the .expected is hand-checked against the LRM (sim2
-- agrees).
package state_param_copy_pkg is
  type counter is protected
    procedure bump;
    procedure via_pkg;
    procedure plain;
    impure function peek return integer;
  end protected;
  procedure run_shared;
  impure function shared_peek return integer;
end package;

package body state_param_copy_pkg is
  procedure helper (x : inout integer);
  procedure add_only (x : inout integer) is
  begin
    x := x + 1000;
  end procedure;

  type counter is protected body
    variable n : integer := 1;
    procedure add (x : inout integer) is
    begin
      x := x + 1;
      report "add n=" & integer'image(n);
    end procedure;
    procedure bump is
    begin
      add(n);
      report "after add n=" & integer'image(n);
    end procedure;
    procedure via_pkg is
    begin
      helper(n);
      report "after helper n=" & integer'image(n);
    end procedure;
    procedure plain is
    begin
      add_only(n);
      report "after add_only n=" & integer'image(n);
    end procedure;
    impure function peek return integer is
    begin
      return n;
    end function;
  end protected body;

  shared variable sv : counter;

  procedure helper (x : inout integer) is
  begin
    x := x + 100;
    report "helper peek=" & integer'image(sv.peek);
  end procedure;

  procedure run_shared is
  begin
    sv.via_pkg;
    sv.plain;
  end procedure;

  impure function shared_peek return integer is
  begin
    return sv.peek;
  end function;
end package body;

use work.state_param_copy_pkg.all;
entity state_param_copy is end entity;
architecture a of state_param_copy is
begin
  process
    variable c : counter;
  begin
    c.bump;
    run_shared;
    assert c.peek = 2 and shared_peek = 1101 report "wrong state after the calls" severity failure;
    wait;
  end process;
end architecture;
