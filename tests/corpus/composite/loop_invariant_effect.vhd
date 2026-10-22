-- difftest: top=loop_invariant_effect exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references fail on
-- the same index with other words).
-- The inner loop reports before it reads `v(k)` with `k` out of range: the report comes out
-- before the failure, also when the loop's invariant code runs before the loop (#526).
entity loop_invariant_effect is end;

architecture a of loop_invariant_effect is
  type ints is array (natural range <>) of integer;

  function total(v : ints; k0 : integer) return integer is
    variable k : integer := k0;
    variable s : integer := 0;
  begin
    for i in 1 to 2 loop
      for j in v'range loop
        report "pass " & integer'image(i) & " " & integer'image(j);
        s := s + v(k) * j;
      end loop;
      k := k + 3;
    end loop;
    return s;
  end function;
begin
  process
    variable v : ints(0 to 3) := (1, 2, 3, 4);
  begin
    report "total " & integer'image(total(v, 0));
    report "total " & integer'image(total(v, 2));
    report "not reached";
    wait;
  end process;
end;
