-- difftest: top=static_intermediate_dynamic exit=1
-- An operand that is not static (a variable exponent) makes each operation checked against
-- INTEGER: 2 ** e with e = 31 overflows at run time (#872), in an elsif condition after a
-- branch with a nested if.
entity static_intermediate_dynamic is
end entity;

architecture a of static_intermediate_dynamic is
begin
  process
    variable e : natural := 31;
    variable n : natural := 0;
  begin
    report integer'image(2 ** (e - 1) - 1 + 2 ** (e - 1));
    if e = 0 then
      if n = 1 then
        n := 2;
      end if;
    elsif n < 2 ** e - 1 then
      n := 1;
    end if;
    report "not reached";
    wait;
  end process;
end architecture;
