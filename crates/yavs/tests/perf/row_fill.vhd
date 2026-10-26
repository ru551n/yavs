-- Rows of bit vectors behind access values, filled one element at a time in nested loops by a
-- procedure declared in the process (the SDRAM model of a SoC testbench does this to initialise
-- its storage, #768). The inner loop reads the access value, its object and the descriptors
-- through the frame of the process; none of that changes in the loop, so it is computed before
-- the loop and the fill runs as one element loop.
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
    variable ones : integer := 0;
    procedure init_row (row : integer) is
    begin
      if bank(row) = null then
        bank(row) := new ram_type;
        for i in (2**col_bits - 1) downto 0 loop
          for j in data_bits downto 0 loop
            bank(row)(i)(j) := '1';
          end loop;
        end loop;
      end if;
    end procedure;
  begin
    for row in 0 to 3 loop
      init_row(row);
      init_row(row);
    end loop;
    for row in 0 to 3 loop
      for i in bank(row)'range loop
        if bank(row)(i)(3) = '1' and bank(row)(i)(data_bits) = '1' then ones := ones + 1; end if;
      end loop;
    end loop;
    assert ones = 4 * 32 report "bad fill" severity failure;
    report "ones=" & integer'image(ones);
    wait;
  end process;
end;
