-- difftest: top=resolved_array_bounds std=2008
-- The array of driving values passed to a resolution function starts at the left bound
-- of the parameter's index subtype (ascending index subtypes here; the descending case is
-- in the discrepancy catalogue, `resolution-array-bounds`).
entity resolved_array_bounds is end entity;
architecture a of resolved_array_bounds is
  type pos_vec is array (positive range <>) of integer;
  type int_vec is array (integer range <>) of integer;
  function sum_pos(x : pos_vec) return integer is
    variable r : integer := 0;
  begin
    assert x'left = 1 and x'right = x'length severity failure;
    for i in 1 to x'length loop
      r := r + x(i);
    end loop;
    return r;
  end function;
  function sum_int(x : int_vec) return integer is
    variable r : integer := 0;
  begin
    assert x'left = integer'low and x'ascending severity failure;
    for i in x'range loop
      r := r + x(i);
    end loop;
    return r;
  end function;
  signal sp : sum_pos integer := 0;
  signal si : sum_int integer := 0;
begin
  sp <= 1;
  sp <= 2;
  si <= 10;
  si <= 20 after 1 ns;
  process
  begin
    wait for 2 ns;
    report "sp = " & integer'image(sp) & ", si = " & integer'image(si);
    wait;
  end process;
end architecture;
