-- LRM 9.3.3.3: a named aggregate with array values under downto range choices and no
-- constrained context: its direction, and so where each value's elements land.
entity top is end entity;
architecture a of top is
  function f(v : bit_vector) return string is
  begin
    return to_string(v) & " " & integer'image(v'left) & ":" & integer'image(v'right);
  end function;
begin
  process
    variable a : bit_vector(1 downto 0) := "01";
    variable b : bit_vector(0 to 1) := "10";
  begin
    report "RES " & f((3 downto 2 => a, 1 => '1', 0 => '0'));
    report "RES " & f((3 downto 2 => a, 1 downto 0 => b));
    wait;
  end process;
end architecture;
