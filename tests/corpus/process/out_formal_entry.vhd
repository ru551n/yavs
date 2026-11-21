-- difftest: top=out_formal_entry std=2008 ref=hand
-- A scalar or access out formal starts at its subtype's default value ('left, null), not the
-- actual's, and copies that back when never assigned (LRM 4.2.2.2, 14.6 b) 5), #1184): by
-- reference, through a temporary (a narrower actual, a 4-byte array element, a logic element)
-- and through a formal conversion. An inout formal still takes the actual's value; a body that
-- reaches the actual by another path sees its old value until the call returns. sim1 starts an
-- out formal at the actual's value instead and rejects the formal conversion; the .expected is
-- hand-checked against the LRM (sim2 agrees).
library ieee;
use ieee.std_logic_1164.all;
entity out_formal_entry is end entity;
architecture a of out_formal_entry is
  subtype s is integer range 3 to 9;
  type acc is access integer;
  procedure rd (variable x : out integer) is begin report "rd x=" & integer'image(x); x := 1; end procedure;
  procedure none (variable x : out integer) is begin end procedure;
  procedure none_s (variable x : out s) is begin end procedure;
  procedure none_r (variable x : out real) is begin end procedure;
  procedure none_b (variable x : out boolean) is begin end procedure;
  procedure none_c (variable x : out character) is begin end procedure;
  procedure none_a (variable x : out acc) is begin end procedure;
  procedure rd_l (variable x : out std_logic) is
  begin
    report "rd_l x=" & std_logic'image(x);
    x := '0';
  end procedure;
  procedure none_l (variable x : out std_logic) is begin end procedure;
  procedure keep (variable x : inout integer) is begin end procedure;
  procedure set (variable x : out integer; v : integer) is begin x := v; end procedure;
  procedure two (variable x : out integer; y : integer) is
  begin
    report "two y=" & integer'image(y);
  end procedure;
begin
  process
    type ia is array (0 to 3) of integer;
    variable a : ia := (others => 42);
    variable v : integer := 5;
    variable w : integer range 0 to 100 := 50;
    variable i : integer := 2;
    variable r : real := 1.5;
    variable b : boolean := true;
    variable c : character := 'q';
    variable p : acc := new integer'(3);
    variable lv : std_logic_vector(0 to 3) := "1111";
    variable rv : real := 1.5;
    procedure seen (variable x : out integer) is
    begin
      if v = 5 then x := 1; else x := 2; end if;
    end procedure;
  begin
    rd(v);
    report "v=" & integer'image(v);
    v := 5;
    none(v);
    report "v=" & integer'image(v);
    rd(w);
    report "w=" & integer'image(w);
    none_s(w);
    report "w=" & integer'image(w);
    rd(a(i));
    report "a=" & integer'image(a(i));
    a(i) := 42;
    none(a(i));
    report "a=" & integer'image(a(i));
    none_r(r);
    report "r'left=" & boolean'image(r = real'left);
    none_b(b);
    none_c(c);
    report "b=" & boolean'image(b) & " c=" & integer'image(character'pos(c));
    none_a(p);
    report "p null=" & boolean'image(p = null);
    rd_l(lv(1));
    none_l(lv(2));
    report "lv=" & std_logic'image(lv(1)) & std_logic'image(lv(2));
    none(real(x) => rv);
    report "rv=" & boolean'image(rv = real(integer'left));
    v := 5;
    keep(v);
    report "keep v=" & integer'image(v);
    seen(v);
    report "seen v=" & integer'image(v);
    v := 5;
    two(v, v + 1);
    report "two v=" & integer'image(v);
    set(a(i), 7);
    set(w, 9);
    report "a=" & integer'image(a(i)) & " w=" & integer'image(w);
    wait;
  end process;
end architecture;
