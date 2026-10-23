-- Nested loops over an array of arrays in a process that does not suspend: the position of the
-- outer index, descriptors and lengths are the same on every pass of the inner loop (loop-invariant
-- code motion).
entity licm_loop is end;
architecture a of licm_loop is
  type row is array (0 to 15) of integer;
  type mat is array (0 to 7) of row;
  type col is array (0 to 7) of integer;
begin
  process
    variable m : mat;
    variable w : col;
    variable acc : integer := 0;
  begin
    for rep in 1 to 10 loop
      for i in mat'range loop
        w(i) := i + rep;
        for j in row'range loop
          m(i)(j) := i * 16 + j + w(i);
        end loop;
      end loop;
      for i in mat'range loop
        for j in row'range loop
          acc := (acc + m(i)(j) * w(i)) mod 100003;
        end loop;
      end loop;
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
