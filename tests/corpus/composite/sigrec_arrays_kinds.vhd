-- difftest: top=sigrec_arrays_kinds std=2008 stop=12ns
-- Every kind of leaf in the elements of an array of records (#677): bit, boolean, enumeration,
-- narrow and wide integers, real, time, bit and integer vectors, and a record inside the record.
library ieee; use ieee.std_logic_1164.all;
package sigrec_kinds_pkg is
  type color is (red, green, blue);
  type ivec is array (0 to 2) of integer;
  type small_t is array (0 to 1) of integer range -5 to 5;
  type inner is record
    b : bit;
    ok : boolean;
  end record;
  type item is record
    bt : bit;
    bo : boolean;
    c : color;
    narrow : integer range 0 to 255;
    wide : integer;
    r : real;
    t : time;
    bv : bit_vector(3 downto 0);
    iv : ivec;
    sm : small_t;
    sl : std_logic;
    inn : inner;
  end record;
  type items is array (natural range <>) of item;
  function mk (i : integer) return item;
  function ri (r : real) return string;
  function img (x : item) return string;
end package;
package body sigrec_kinds_pkg is
  function mk (i : integer) return item is
    variable x : item;
  begin
    x.bt := '1' when i mod 2 = 1 else '0';
    x.bo := i mod 3 = 0;
    x.c := color'val(i mod 3);
    x.narrow := (i * 37) mod 256;
    x.wide := i * 10000007 - 3;
    x.r := real(i) * 1.5;
    x.t := i * 1 ns;
    for k in 0 to 3 loop
      x.bv(k) := '1' when ((i / (2 ** k)) mod 2) = 1 else '0';
    end loop;
    x.iv := (i, -i, i * 3);
    x.sm := (i mod 5, -(i mod 5));
    x.sl := '1' when i mod 2 = 0 else 'Z';
    x.inn.b := '1' when i mod 4 = 0 else '0';
    x.inn.ok := i mod 2 = 0;
    return x;
  end;
  function ri (r : real) return string is
  begin
    if abs(r) < 1.0e6 then
      return integer'image(integer(r * 2.0));
    end if;
    return "big";
  end;
  function img (x : item) return string is
  begin
    return bit'image(x.bt) & boolean'image(x.bo) & color'image(x.c) & integer'image(x.narrow) & "/" &
      integer'image(x.wide) & "/" & ri(x.r) & "/" & time'image(x.t) & "/" &
      to_string(x.bv) & "/" & integer'image(x.iv(0)) & integer'image(x.iv(1)) & integer'image(x.iv(2)) & "/" &
      integer'image(x.sm(0)) & integer'image(x.sm(1)) & "/" & std_logic'image(x.sl) & "/" &
      bit'image(x.inn.b) & boolean'image(x.inn.ok);
  end;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.sigrec_kinds_pkg.all;
entity sigrec_arrays_kinds is end;
architecture a of sigrec_arrays_kinds is
  signal s : items(3 downto 0);
  signal u : items(0 to 2);
begin
  d1 : process
  begin
    wait for 1 ns;
    for i in s'range loop
      s(i) <= mk(i + 7);
    end loop;
    wait;
  end process;

  d2 : process
    variable v : items(0 to 2);
  begin
    wait for 2 ns;
    for i in v'range loop
      v(i) := mk(20 + i);
    end loop;
    u <= v;
    wait;
  end process;

  chk : process
    variable v : items(3 downto 0);
    variable w : items(0 to 2);
  begin
    for step in 1 to 5 loop
      wait for 1.5 ns;
      v := s;
      w := u;
      for i in v'range loop
        report integer'image(step) & " s" & integer'image(i) & "=" & img(v(i));
      end loop;
      for i in w'range loop
        report integer'image(step) & " u" & integer'image(i) & "=" & img(w(i));
      end loop;
      report integer'image(step) & " e=" & img(s(2)) & " " & img(u(1));
    end loop;
    wait;
  end process;
end;
