-- difftest: top=case_generate_array std=2008
-- A case generate over an array: choices match by length and elements, whatever the bounds
-- and direction of the expression and of the choice (LRM 9.2.3, 11.8).
entity case_generate_array is
  generic (VAL : natural := 2);
end entity;

architecture a of case_generate_array is
  function to_bv (v : natural) return bit_vector is
    variable r : bit_vector(1 downto 0);
    variable v1 : natural := v;
  begin
    for i in r'reverse_range loop
      if v1 mod 2 = 1 then r(i) := '1'; else r(i) := '0'; end if;
      v1 := v1 / 2;
    end loop;
    return r;
  end function;
  constant B : bit_vector(1 downto 0) := to_bv(VAL);
  constant C : bit_vector(0 to 1) := "01";
begin
  g1 : case B generate
    when "00" => assert false report "g1 zero" severity note;
    when "10" => assert false report "g1 two" severity note;
    when others => assert false report "g1 others" severity note;
  end generate;
  g2 : case C generate
    when "00" => assert false report "g2 zero" severity note;
    when "01" => assert false report "g2 one" severity note;
    when others => assert false report "g2 others" severity note;
  end generate;
end architecture;
