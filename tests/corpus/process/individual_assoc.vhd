-- difftest: top=individual_assoc std=2008
-- Individual association of subprogram parameters element by element (LRM 6.5.7.1): an
-- unconstrained formal takes its bounds from the parts, out and inout parts are copied back.
entity individual_assoc is end;
architecture t of individual_assoc is
  signal tl : bit := '0';

  procedure set_left(signal x : out bit_vector) is
  begin
    x(x'left) <= '1';
  end procedure;

  procedure show(signal x : in bit_vector) is
  begin
    report "show " & integer'image(x'left) & " " & integer'image(x'right) & " "
      & to_string(x) & " " & boolean'image(x'event);
  end procedure;

  procedure swap(variable v : inout integer_vector) is
    variable t : integer;
  begin
    report "swap " & integer'image(v'left) & " to " & integer'image(v'right);
    t := v(v'left);
    v(v'left) := v(v'right);
    v(v'right) := t;
  end procedure;

  function total(x : integer_vector) return integer is
    variable s : integer := 0;
  begin
    for i in x'range loop
      s := s + x(i) * i;
    end loop;
    return s;
  end function;

  function bits(x : bit_vector) return string is
  begin
    return integer'image(x'left) & ":" & to_string(x);
  end function;
begin
  process
    variable a, b : integer;
  begin
    set_left(x(0) => tl);
    show(x(1) => tl);
    a := 1;
    b := 2;
    swap(v(4) => a, v(5) => b);
    assert a = 2 and b = 1 report "swap failed" severity failure;
    report "total " & integer'image(total(x(2) => 10, x(3) => a));
    report "bits " & bits(x(1) => '1', x(0) => '0');
    wait for 1 ns;
    assert tl = '1' report "set_left failed" severity failure;
    show(x(7) => tl);
    wait on tl for 1 ns;
    report "done";
    wait;
  end process;
end architecture;
