-- difftest: top=array_param_cells_slice std=2008 exit=1 ignore=fatalmsg,path
-- ignore=fatalmsg,path: the references word the error and name its process differently.
-- A slice outside a parameter's bounds.
entity array_param_cells_slice is end;
architecture t of array_param_cells_slice is
  function cut(x : bit_vector; h, l : integer) return bit_vector is
  begin
    return x(h downto l);
  end;
  function len(x : bit_vector) return integer is
  begin
    return x'length;
  end;
begin
  process
    variable a : bit_vector(7 downto 2) := "101010";
  begin
    for i in 3 to 9 loop
      report integer'image(len(cut(a, i, 2)));
    end loop;
    wait;
  end process;
end;
