-- difftest: top=row_fill std=2008
-- Rows of bit vectors behind access values, filled and read back element by element in nested
-- loops (the memory models of a SoC testbench do this to initialise their storage); loops over
-- an empty range never touch their access value, which may be null.
entity row_fill is end;
architecture t of row_fill is
  constant col_bits  : integer := 5;
  constant data_bits : integer := 7;
  type ram_type is array (2**col_bits - 1 downto 0) of bit_vector (data_bits downto 0);
  type ram_pntr is access ram_type;
  type ram_stor is array (3 downto 0) of ram_pntr;
begin
  process
    variable bank : ram_stor;
    variable ones, zeros : integer := 0;
    variable empty : ram_pntr := null;
  begin
    for row in 0 to 2 loop
      if bank(row) = null then
        bank(row) := new ram_type;
        for i in (2**col_bits - 1) downto 0 loop
          for j in data_bits downto 0 loop
            bank(row)(i)(j) := '1';
          end loop;
        end loop;
      end if;
    end loop;
    -- a second fill that changes some elements back
    for i in 3 to 9 loop
      for j in 2 to 5 loop
        bank(1)(i)(j) := '0';
      end loop;
    end loop;
    -- an empty range never reads the null access value
    for i in 1 to 0 loop
      for j in data_bits downto 0 loop
        empty(i)(j) := '1';
      end loop;
    end loop;
    for row in 0 to 3 loop
      if bank(row) /= null then
        for i in bank(row)'range loop
          for j in bank(row)(i)'range loop
            if bank(row)(i)(j) = '1' then ones := ones + 1; else zeros := zeros + 1; end if;
          end loop;
        end loop;
      end if;
    end loop;
    report "ones=" & integer'image(ones) & " zeros=" & integer'image(zeros);
    assert ones = 3 * 32 * 8 - 7 * 4 and zeros = 7 * 4 severity failure;
    wait;
  end process;
end;
