-- difftest: top=lget_reuse std=2008
-- The same element of a logic vector read twice: the second read gives the first's value only
-- while nothing can have written the vector: a write of it, of a slice, through an alias or a
-- pointer, by a nested subprogram, a call or a suspension.
library ieee;
use ieee.std_logic_1164.all;

entity lget_reuse is
end entity;

architecture a of lget_reuse is
  subtype w8 is std_logic_vector(7 downto 0);
  type slv_ptr is access std_logic_vector;
  signal s : w8 := x"a5";

  function table(p : w8; d : std_logic_vector(3 downto 0)) return w8 is
    variable r : w8;
    variable e : std_logic_vector(3 downto 0);
  begin
    e := d(0) & d(1) & d(2) & d(3);
    r(0) := e(1) xor e(0) xor p(6) xor p(7);
    r(1) := e(2) xor e(1) xor p(5) xor p(7);
    r(2) := e(3) xor e(0) xor p(6) xor p(5);
    r(3) := e(1) xor e(2) xor p(1) xor p(7);
    r(7 downto 4) := p(3 downto 0);
    return r;
  end function;

  function write_between(p : w8) return w8 is
    variable v : w8 := p;
    variable x, y : std_logic;
  begin
    x := v(1);
    v(1) := not v(1);
    y := v(1);
    v(0) := x xor y;
    v(7 downto 4) := v(3 downto 0);
    v(6) := v(5) xor v(1);
    return v;
  end function;

  function through_alias(p : w8) return w8 is
    variable v : w8 := p;
    alias a : std_logic_vector(0 to 7) is v;
    variable x : std_logic;
  begin
    x := v(7);
    a(0) := not x;
    v(0) := v(7) xor x;
    return v;
  end function;

  function through_pointer(p : w8) return w8 is
    variable q, r : slv_ptr;
    variable x : std_logic;
    variable o : w8;
  begin
    q := new std_logic_vector'(p);
    r := q;
    x := q.all(2);
    r.all(2) := not x;
    o := q.all;
    o(0) := q.all(2) xor x;
    deallocate(q);
    return o;
  end function;

  function by_nested(p : w8) return w8 is
    variable v : w8 := p;
    variable x : std_logic;
    procedure flip is
    begin
      v(3) := not v(3);
    end procedure;
  begin
    x := v(3);
    flip;
    v(0) := v(3) xor x;
    return v;
  end function;

  procedure by_inout(variable v : inout w8; x : out std_logic) is
    variable t : std_logic;
  begin
    t := v(4);
    v(4) := not t;
    x := v(4) xor t;
  end procedure;

  function in_loop(p : w8) return w8 is
    variable v : w8 := p;
  begin
    for i in 1 to 7 loop
      v(0) := v(0) xor v(1);
      v(1) := v(1) xor v(0);
    end loop;
    return v;
  end function;
begin
  process
    variable v : w8 := x"3c";
    variable x, y : std_logic;
  begin
    report to_string(table(x"96", x"5"));
    report to_string(write_between(x"96"));
    report to_string(through_alias(x"96"));
    report to_string(through_pointer(x"96"));
    report to_string(by_nested(x"96"));
    report to_string(in_loop(x"96"));
    by_inout(v, x);
    report to_string(v) & " " & to_string(x);
    x := s(0);
    s <= x"5a";
    y := s(0);
    wait for 1 ns;
    report to_string(x) & to_string(y) & to_string(s(0));
    x := v(2);
    v(2) := not v(2);
    wait for 1 ns;
    y := v(2);
    report to_string(x) & to_string(y);
    wait;
  end process;
end architecture;
