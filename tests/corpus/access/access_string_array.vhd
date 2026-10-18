-- difftest: top=access_string_array std=2008
entity access_string_array is end;
architecture t of access_string_array is
  type str_ptr is access string;
  type int_vec is array (natural range <>) of integer;
  type vec_ptr is access int_vec;
begin
  process
    variable s : str_ptr;
    variable v : vec_ptr;
    variable copy : str_ptr;
  begin
    s := new string'("hello");
    copy := s;
    s(1) := 'J';
    report copy.all & " len=" & integer'image(s'length);
    deallocate(s);
    v := new int_vec(1 to 4);
    v.all := (1, 2, 3, 4);
    v(2) := 20;
    report integer'image(v(1) + v(2) + v(3) + v(4)) & " high=" & integer'image(v'high);
    v := new int_vec'(v.all & v.all);
    report "doubled len=" & integer'image(v'length);
    wait;
  end process;
end;
