-- difftest: top=static_intermediate_param exit=1
-- A subprogram parameter is not known at elaboration: p ** 31 - 1 checks each operation and
-- fails for p = 2 (#872), while a loop over in-range exponents does not. A generate parameter
-- is checked the same way (tests/discrepancies/static-intermediate-generate-parameter).
entity static_intermediate_param is
end entity;

architecture a of static_intermediate_param is
  function f(p : integer) return integer is
  begin
    return p ** 31 - 1;
  end function;
begin
  process
  begin
    for i in 29 to 30 loop
      report integer'image(2 ** i - 1);
    end loop;
    report integer'image(f(1));
    report integer'image(f(2));
    report "not reached";
    wait;
  end process;
end architecture;
