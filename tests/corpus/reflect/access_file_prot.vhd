-- difftest: top=access_file_prot std=2019 ref=hand
-- std.reflection access, file and protected mirrors (LRM 2019 16.11).
-- ref=hand: the reference simulator 1.23 returns the pointer, not 42, for get of an access to INTEGER.
use std.reflection.all;
use std.textio.all;
entity access_file_prot is end entity;

architecture test of access_file_prot is
  type int_ptr is access integer;
  type str_ptr is access string;
  type rec is record
    a : integer;
  end record;
  type rec_ptr is access rec;
  type int_file is file of integer;
  type counter is protected
    procedure inc;
  end protected;
  type counter is protected body
    variable n : integer := 0;
    procedure inc is begin n := n + 1; end procedure;
  end protected body;
  shared variable sc : counter;
begin
  process
    variable p : int_ptr := new integer'(42);
    variable q : int_ptr;
    variable s : str_ptr := new string'("hey");
    variable rp : rec_ptr := new rec'(a => 3);
    variable am : access_subtype_mirror;
    variable av : access_value_mirror;
    file f : int_file open write_mode is "reflect_af.bin";
    variable fm : file_value_mirror;
    variable pc : counter;
  begin
    av := p'reflect.to_access;
    am := av.get_subtype_mirror;
    report "p null " & boolean'image(av.is_null) & " val "
      & integer'image(av.get.to_integer.value) & " sub " & am.simple_name
      & " des " & am.designated_subtype.simple_name;
    assert p'reflect.get_value_class = class_access severity failure;
    assert am.to_subtype_mirror.get_type_class = class_access severity failure;
    assert av.to_value_mirror.get_value_class = class_access severity failure;
    report "q null " & boolean'image(q'reflect.to_access.is_null);
    report "s " & s'reflect.to_access.get.to_array.get(2).to_enumeration.image & " len "
      & index'image(s'reflect.to_access.get.to_array.get_subtype_mirror.length);
    report "rp " & integer'image(rp'reflect.to_access.get.to_record.get("a").to_integer.value)
      & " des " & rec_ptr'reflect.to_access.designated_subtype.simple_name;
    report "line des " & line'reflect.to_access.designated_subtype.simple_name;
    fm := f'reflect.to_file;
    report "f name " & fm.get_file_logical_name & " kind "
      & file_open_kind'image(fm.get_file_open_kind) & " sub " & fm.get_subtype_mirror.simple_name
      & " des " & fm.get_subtype_mirror.designated_subtype.simple_name;
    assert f'reflect.get_value_class = class_file severity failure;
    assert int_file'reflect.get_type_class = class_file severity failure;
    report "text des " & text'reflect.to_file.designated_subtype.simple_name;
    assert pc'reflect.get_value_class = class_protected severity failure;
    report "pc " & pc'reflect.to_protected.get_subtype_mirror.simple_name & " "
      & counter'reflect.to_protected.simple_name & " "
      & sc'reflect.get_subtype_mirror.simple_name;
    assert counter'reflect.get_type_class = class_protected severity failure;
    assert pc'reflect.to_protected.to_value_mirror.get_value_class = class_protected
      severity failure;
    file_close(f);
    report "done";
    wait;
  end process;
end architecture;
