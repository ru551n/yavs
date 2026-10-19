-- difftest: top=enums std=2019 ref=hand
-- std.reflection enumeration mirrors (LRM 2019 16.11): subtypes, bounds, literals by index
-- and name, to_value_mirror round trips, character and std_ulogic literals.
-- ref=hand: the reference simulator 1.23 ignores the range of an enumeration subtype (mid and rev
-- report all four literals) and looks literal names up case-sensitively.
library ieee;
use ieee.std_logic_1164.all;
use std.reflection.all;
entity enums is end entity;

architecture test of enums is
  type color is (red, Green, blue, white);
  subtype mid is color range green to blue;
  subtype rev is color range blue downto red;
begin
  process
    variable c : mid := blue;
    variable sl : std_ulogic := 'Z';
    variable ch : character := 'A';
    variable em : enumeration_subtype_mirror;
    variable ev : enumeration_value_mirror;
    variable vm : value_mirror;
    variable sm : subtype_mirror;
  begin
    em := mid'reflect.to_enumeration;
    report "mid " & em.simple_name & " len " & index'image(em.length)
      & " left " & em.left.image & " right " & em.right.image
      & " low " & em.low.image & " high " & em.high.image
      & " asc " & boolean'image(em.ascending);
    em := rev'reflect.to_enumeration;
    report "rev " & em.simple_name & " len " & index'image(em.length)
      & " left " & em.left.image & " right " & em.right.image
      & " low " & em.low.image & " high " & em.high.image
      & " asc " & boolean'image(em.ascending);
    em := color'reflect.to_enumeration;
    report "color lit0 " & em.enumeration_literal(0).image & " lit3 "
      & em.enumeration_literal(3).image & " byname " & integer'image(em.enumeration_literal("WHITE").pos);
    ev := c'reflect.to_enumeration;
    report "c pos " & integer'image(ev.pos) & " image " & ev.image
      & " sub " & ev.get_subtype_mirror.simple_name;
    vm := ev.to_value_mirror;
    assert vm.get_value_class = class_enumeration severity failure;
    report "c roundtrip " & vm.to_enumeration.image & " " & vm.get_subtype_mirror.simple_name;
    sm := em.to_subtype_mirror;
    assert sm.get_type_class = class_enumeration severity failure;
    report "color sm " & sm.simple_name & " " & sm.to_enumeration.high.image;
    report "sl " & sl'reflect.to_enumeration.image & " pos "
      & integer'image(sl'reflect.to_enumeration.pos) & " sub "
      & sl'reflect.get_subtype_mirror.simple_name & " len "
      & index'image(std_ulogic'reflect.to_enumeration.length);
    report "ch " & ch'reflect.to_enumeration.image & " pos "
      & integer'image(ch'reflect.to_enumeration.pos) & " len "
      & index'image(character'reflect.to_enumeration.length);
    report "bit " & bit'reflect.to_enumeration.enumeration_literal(1).image
      & " sev " & severity_level'reflect.to_enumeration.high.image;
    report "done";
    wait;
  end process;
end architecture;
