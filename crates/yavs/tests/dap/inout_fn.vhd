-- An impure function with an inout parameter (VHDL-2019), refused by conditions (#364).
entity t3 is end;
architecture a of t3 is
  signal s : integer := 4;
  impure function take(x : inout integer) return integer is
  begin
    x := x + 1;
    return x + s;
  end function;
begin
  p : process
    variable v : integer := 1;
  begin
    wait for 1 ns;
    report "here";
    wait;
  end process;
end;
