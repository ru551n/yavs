-- difftest: top=row_fill_null std=2008 exit=1
-- The access value of a row is null when the inner loop starts: the loop body reports first,
-- then dereferences, so the failure comes after the report whatever the loops hoist.
entity row_fill_null is end;
architecture t of row_fill_null is
  type vec_t is array (3 downto 0) of bit_vector (7 downto 0);
  type vec_p is access vec_t;
  type vec_s is array (1 downto 0) of vec_p;
begin
  process
    variable bank : vec_s;
  begin
    bank(0) := new vec_t;
    for row in 0 to 1 loop
      for i in 3 downto 0 loop
        for j in 7 downto 0 loop
          if i = 3 and j = 7 then report "row " & integer'image(row); end if;
          bank(row)(i)(j) := '1';
        end loop;
      end loop;
    end loop;
    report "unreachable";
    wait;
  end process;
end;
