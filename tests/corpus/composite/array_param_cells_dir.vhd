-- difftest: top=array_param_cells_dir std=2008 exit=1 ignore=fatalmsg,path
-- ignore=fatalmsg,path: the references word the error and name its process differently.
-- A slice in the other direction than the parameter's: the direction check, not null (LRM 8.5).
entity array_param_cells_dir is end;
architecture t of array_param_cells_dir is
  function cut(x : bit_vector; a, b : integer) return integer is
  begin
    return x(a to b)'length;
  end;
begin
  process
    variable a : bit_vector(0 to 7) := "10101010";
    variable d : bit_vector(7 downto 0) := "10101010";
  begin
    report integer'image(cut(a, 1, 3));
    report integer'image(cut(a, 3, 1));
    -- Null in the other direction: nothing to check.
    report integer'image(cut(d, 3, 1));
    report integer'image(cut(d, 1, 3));
    wait;
  end process;
end;
