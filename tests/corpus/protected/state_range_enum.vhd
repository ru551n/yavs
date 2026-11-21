-- difftest: top=state_range_enum std=2008 exit=1 ref=hand
-- ref=hand: sim1 checks against the bound's current value (discrepancy
-- `object-range-bound-variable`) and accepts `'g'`; sim2 agrees with this output.
-- A scalar state variable of a protected type body with an enumeration range that reads an
-- earlier state variable: the object keeps the bounds as words, whatever the size of the
-- variable's own type (#1185).
entity state_range_enum is
end entity;
architecture a of state_range_enum is
  type pt is protected
    procedure set (x : character);
    impure function get return string;
  end protected;
  type pt is protected body
    variable hi : character := 'f';
    variable c : character range 'b' to hi := 'c';
    variable e : character range 'a' to hi;
    variable b : boolean range false to (hi > 'e');
    procedure set (x : character) is
    begin
      hi := 'z';
      c := x;
    end procedure;
    impure function get return string is
    begin
      return "c=" & c & " e=" & e & " b=" & boolean'image(b);
    end function;
  end protected body;
  shared variable sp : pt;
begin
  process
  begin
    report sp.get;
    sp.set('f');
    report sp.get;
    sp.set('g');
    report "not reached";
    wait;
  end process;
end architecture;
