-- difftest: top=generics std=2019 ref=hand
-- std.reflection with subtypes whose bounds are known only at elaboration (generics), ports,
-- package objects, generic subprogram instances, and value snapshots (LRM 2019 16.11).
-- ref=hand: the reference simulator 1.23 returns garbage for mirrors of signals and crashes on a
-- mirror of a port.
use std.reflection.all;
package refl_pkg is
  constant pc : integer := 12;
  impure function name_of generic (type t) parameter (x : t) return string;
end package;
package body refl_pkg is
  impure function name_of generic (type t) parameter (x : t) return string is
  begin
    return x'reflect.get_subtype_mirror.simple_name & "/"
      & value_class'image(x'reflect.get_value_class);
  end function;
end package body;

use std.reflection.all;
use work.refl_pkg.all;
entity gchild is
  generic (n : positive; lo : integer; hi_r : real; last : character);
  port (p : in bit_vector(n - 1 downto 0));
end entity;
architecture test of gchild is
  subtype word is bit_vector(n - 1 downto 0);
  subtype rng is integer range lo to lo + n;
  subtype frac is real range 0.0 to hi_r;
  subtype letters is character range 'a' to last;
  type rng_arr is array (rng range <>) of letters;
  subtype rng_arr2 is rng_arr(lo to lo + 1);

  impure function local_high(m : natural) return integer is
    subtype upto is natural range 0 to m;
  begin
    return upto'reflect.to_integer.high.value;
  end function;
  type rec is record
    w : word;
    k : rng;
  end record;
  function int_name is new name_of generic map (t => integer);
  function rec_name is new name_of generic map (t => rec);
  signal ts : time := 3 ns;
  signal sr : rec := ("0110", lo + 2);
begin
  process
    variable r : rec := ((others => '1'), lo + 1);
    variable k : rng := lo;
    variable vm : value_mirror;
  begin
    wait for 1 ns;
    report "word " & index'image(word'reflect.to_array.length) & " "
      & index'image(word'reflect.to_array.left);
    report "rng " & integer'image(rng'reflect.to_integer.low.value) & " "
      & integer'image(rng'reflect.to_integer.high.value) & " "
      & integer'image(k'reflect.to_integer.get_subtype_mirror.high.value);
    report "r.k " & integer'image(r'reflect.to_record.get("k").to_integer.value)
      & " w " & index'image(r'reflect.to_record.get("w").to_array.get_subtype_mirror.length);
    report "p " & p'reflect.to_array.get(0).to_enumeration.image & " "
      & index'image(p'reflect.to_array.get_subtype_mirror.length);
    report "nested " & integer'image(r'reflect.to_record.get("k").get_subtype_mirror.to_integer.high.value)
      & " " & integer'image(rec'reflect.to_record.element_subtype("k").to_integer.low.value)
      & " " & index'image(rec'reflect.to_record.element_subtype(1).to_integer.length);
    assert frac'reflect.to_floating.high.value = hi_r severity failure;
    report "letters " & letters'reflect.to_enumeration.high.image & " "
      & index'image(letters'reflect.to_enumeration.length);
    report "rng_arr " & rng_arr2'reflect.to_array.index_subtype.to_integer.high.image & " "
      & rng_arr2'reflect.to_array.element_subtype.to_enumeration.right.image;
    report "local " & integer'image(local_high(7)) & " " & integer'image(local_high(3));
    report "ts " & integer'image(ts'reflect.to_physical.value) & " " & ts'reflect.to_physical.image
      & " sr " & integer'image(sr'reflect.to_record.get("k").to_integer.value) & " "
      & sr'reflect.to_record.get("w").to_array.get(2).to_enumeration.image;
    report "pc " & pc'reflect.to_integer.image;
    report "inst " & int_name(5) & " " & rec_name(r);
    vm := k'reflect;
    k := lo + 3;
    report "snapshot " & vm.to_integer.image & " now " & k'reflect.to_integer.image;
    wait;
  end process;
end architecture;

entity generics is end entity;
architecture test of generics is
  signal s : bit_vector(3 downto 0) := "0101";
begin
  u : entity work.gchild generic map (n => 4, lo => -2, hi_r => 0.5, last => 'd') port map (p => s);
end architecture;
