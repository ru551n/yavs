entity output_order is end;
architecture t of output_order is
begin
  process
    variable a : integer_vector(1 to 3);
    variable i : integer := 5;
  begin
    for k in 1 to 2000 loop
      report "line " & integer'image(k);
    end loop;
    a(i) := 1;
    wait;
  end process;
end;
