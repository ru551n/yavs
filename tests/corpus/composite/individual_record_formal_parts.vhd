-- difftest: top=individual_record_formal_parts std=2008 ref=hand
-- Unconstrained record formals in parts below a field: the slice and index forms, a nested
-- record, an inout formal. ref=hand: neither reference simulator runs this form of association
-- (the first does not support it, the second stops with an internal error).
entity individual_record_formal_parts is end entity;
architecture behav of individual_record_formal_parts is
  type my_rec is record a : bit; w : bit_vector; end record;
  type nest is record h : integer; r : my_rec; s : string; end record;
  procedure check (v : my_rec) is
  begin
    report bit'image(v.a) & " " & to_string(v.w) & " " & integer'image(v.w'left);
  end procedure;
  procedure ch2 (n : nest) is
  begin
    report integer'image(n.h) & " " & to_string(n.r.w) & " " & integer'image(n.r.w'left) & " " & n.s;
  end procedure;
  procedure vp (variable x : inout my_rec) is begin x.w(x.w'left) := '1'; x.a := '1'; end procedure;
  signal q : bit_vector(4 downto 1) := "1010";
begin
  process
    variable y : bit_vector(0 to 1) := "00";
    variable z : bit;
    variable l : bit_vector(3 to 5) := "010";
  begin
    check (v.a => bit'('1'), v.w(2) => bit'('1'), v.w(3) => bit'('0'));
    check (v.w(6 to 7) => bit_vector'("10"), v.a => q(1));
    ch2 (n.h => 4, n.r.a => bit'('1'), n.r.w => l, n.s => "hello");
    ch2 (n.h => 4, n.r => (a => '0', w => "11"), n.s(1 to 2) => "xy");
    vp (x.a => z, x.w => y);
    report bit'image(z) & " " & to_string(y);
    wait;
  end process;
end architecture;
