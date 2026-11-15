-- difftest: top=assign_target_first std=2008
-- A variable assignment to a composite target named with an index (an element of an array of
-- vectors, of records, of arrays of vectors, a conditional assignment) evaluates the target name
-- and its index before the value (#1156). The order is traced by impure functions in the index
-- and in the value, in a loop long enough to be compiled; a vector field and a slice are assigned
-- too. (A scalar target is left out: the reference simulators differ there, and so a slice bound
-- that is an impure call: one evaluates it more than once.)
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity assign_target_first is
end entity;

architecture sim of assign_target_first is
  subtype u12 is unsigned(11 downto 0);
  type uv is array (natural range <>) of u12;
  type rec is record
    a : integer;
    f : u12;
  end record;
  type rv is array (natural range <>) of rec;
  type uvv is array (natural range <>) of uv(0 to 2);
begin
  process is
    variable w : uv(0 to 3);
    variable ra : rv(0 to 3);
    variable r : rec;
    variable m : uvv(0 to 1);
    variable trace : integer := 0;
    variable verbose : boolean := true;
    variable sum : integer := 0;

    procedure note(c : character) is
    begin
      trace := (trace * 7 + character'pos(c)) mod 1000003;
      if verbose then
        report "event " & c;
      end if;
    end procedure;

    impure function t(k : natural) return natural is
    begin
      note('t');
      return k;
    end function;

    impure function fv(k : natural) return u12 is
    begin
      note('v');
      return to_unsigned(k mod 4096, 12);
    end function;

    impure function fr(k : natural) return rec is
    begin
      note('r');
      return (k, to_unsigned(k mod 4096, 12));
    end function;

    impure function fw(k : natural) return uv is
    begin
      note('w');
      return (to_unsigned(k mod 4096, 12), to_unsigned((k + 1) mod 4096, 12), to_unsigned((k + 2) mod 4096, 12));
    end function;

    impure function c(k : natural) return boolean is
    begin
      note('c');
      return k mod 2 = 0;
    end function;

    procedure step(i : natural) is
    begin
      w(t(i mod 4)) := fv(i);
      ra(t((i + 1) mod 4)) := fr(i);
      r.f := fv(i + 3);
      m(t(i mod 2)) := fw(i);
      m(t((i + 1) mod 2))(t(i mod 3)) := fv(i + 5);
      w(t((i + 2) mod 4)) := fv(i) when c(i) else fv(i + 1);
      w(i mod 2 to i mod 2 + 2) := fw(i + 7);
    end procedure;
  begin
    for i in 0 to 1999 loop
      step(i);
      verbose := false;
      sum := (sum + to_integer(w(i mod 4)) + ra(i mod 4).a + to_integer(m(i mod 2)(i mod 3))) mod 1000003;
    end loop;
    report "trace=" & integer'image(trace) & " sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
