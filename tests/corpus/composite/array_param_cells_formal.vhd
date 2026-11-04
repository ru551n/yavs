-- difftest: top=array_param_cells_formal std=2008 exit=1 ignore=fatalmsg,path
-- ignore=fatalmsg,path: the references word the error and name its process differently.
-- An actual of another length than a constrained formal with bounds from a generic.
entity array_param_cells_formal is
  generic (g : integer := 4);
end;
architecture t of array_param_cells_formal is
  function f(x : bit_vector(g - 1 downto 0)) return integer is
  begin
    return x'length;
  end;
begin
  process
    variable a : bit_vector(7 downto 0) := "10101010";
    variable n : integer := 3;
  begin
    report integer'image(f(a(n downto 0)));
    n := 2;
    report integer'image(f(a(n downto 0)));
    wait;
  end process;
end;
