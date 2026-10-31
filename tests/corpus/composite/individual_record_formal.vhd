-- difftest: top=individual_record_formal std=2008
-- An unconstrained record formal associated in parts takes its bounds from them (LRM 6.5.7.1).
entity individual_record_formal is end entity;
architecture behav of individual_record_formal is
  type my_rec is record a : bit; w : bit_vector; end record;
  procedure check (v : my_rec) is
  begin
    report bit'image(v.a) & " " & to_string(v.w) & " " & integer'image(v.w'left);
  end procedure;
  procedure pack (a : bit; w : bit_vector) is begin check (v.a => a, v.w => w); end procedure;
  signal q : bit_vector(4 downto 1) := "1010";
begin
  process
    variable z : bit;
    variable l : bit_vector(3 to 5) := "010";
  begin
    pack ('0', "01");
    pack ('1', l);
    check (v.a => z, v.w => q);
    wait;
  end process;
end architecture;
