-- difftest: top=array_param_cells_result std=2008 exit=1 ref=hand ignore=fatalmsg,path
-- ref=hand: sim1 runs this to the end with no error, sim2 fails; the value of a return statement
-- must belong to the return type mark's subtype (LRM 10.13). ignore=fatalmsg,path: the wording.
-- A result of another length than the function's constrained return subtype, with bounds from a
-- generic.
entity array_param_cells_result is
  generic (g : integer := 4);
end;
architecture t of array_param_cells_result is
  subtype nib is bit_vector(g - 1 downto 0);
  function cut(x : bit_vector; h : integer) return nib is
  begin
    return x(h downto 0);
  end;
begin
  process
    variable a : bit_vector(7 downto 0) := "10101010";
    variable r : nib;
  begin
    r := cut(a, 3);
    report integer'image(r'length);
    r := cut(a, 4);
    report integer'image(r'length);
    wait;
  end process;
end;
