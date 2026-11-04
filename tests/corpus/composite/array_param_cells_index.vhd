-- difftest: top=array_param_cells_index std=2008 exit=1 ignore=fatalmsg,path
-- ignore=fatalmsg,path: the references word the error and name its process differently.
-- An index outside a parameter's bounds, on a run-time position, in a loop that gets compiled.
entity array_param_cells_index is end;
architecture t of array_param_cells_index is
  function at_pos(x : bit_vector; i : integer) return bit is
  begin
    return x(i);
  end;
begin
  process
    variable a : bit_vector(7 downto 2) := "101010";
    variable b : bit;
  begin
    for i in 7 downto 0 loop
      b := at_pos(a, i);
      report bit'image(b);
    end loop;
    wait;
  end process;
end;
