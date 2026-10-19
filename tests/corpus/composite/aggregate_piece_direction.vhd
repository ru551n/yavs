-- difftest: top=aggregate_piece_direction std=2008
-- 2008 array values under range choices in a named aggregate without a constrained context
-- (#230): the aggregate takes the direction of those choices, not of the index subtype, and
-- each value fills its range from the left. Range choices of elements keep any direction.
-- Static and non-static choices, logic and integer arrays, constants, function arguments,
-- concatenation operands and signal values.
entity aggregate_piece_direction is end;
architecture t of aggregate_piece_direction is
  type iv is array (natural range <>) of integer;
  function f(v : bit_vector) return string is
  begin
    return to_string(v) & " " & integer'image(v'left) & ":" & integer'image(v'right)
      & " " & boolean'image(v'ascending);
  end;
  function fi(v : iv) return string is
  begin
    return integer'image(v(v'left)) & "," & integer'image(v(v'right)) & " "
      & integer'image(v'left) & ":" & integer'image(v'right);
  end;
  constant c : bit_vector := (3 downto 2 => "01", 1 downto 0 => "10");
  signal sa : bit_vector(1 downto 0) := "01";
  signal sb : bit_vector(0 to 1) := "10";
begin
  process
    variable a : bit_vector(1 downto 0) := "01";
    variable b : bit_vector(0 to 1) := "10";
    variable x : iv(0 to 1) := (1, 2);
    variable k : natural := 4;
  begin
    report f((3 downto 2 => a, 1 downto 0 => b));
    report f((3 downto 2 => a, 1 => '1', 0 => '0'));
    report f((3 => '1', 2 downto 1 => a, 0 => '0'));
    report f((3 downto 2 => a, 1 downto 0 => '0'));
    report f((1 downto 0 => '0', 3 downto 2 => a));
    report f((3 downto 2 => a, 0 to 1 => '0'));
    report f((1 to 2 => a, 3 to 4 => b));
    report f((1 downto 0 => a));
    report f((k downto k - 1 => a));
    report f((k to k + 1 => a));
    report fi((5 downto 4 => x, 3 => 7));
    report f(c) & " " & f(bit_vector'(3 downto 2 => a, 1 downto 0 => b) & a);
    report f((3 downto 2 => sa, 1 downto 0 => sb));
    -- Element range choices alone keep the index subtype's direction.
    report f((3 downto 2 => '1', 1 downto 0 => '0'));
    assert c = "0110" and c'left = 3 severity failure;
    wait;
  end process;
end;
