-- Functions that run on native frames (called from compiled code without the interpreter's
-- frame): small numeric functions with loops and a nested subprogram.
entity llvm_frames is end;
architecture a of llvm_frames is
  function collatz(n : natural) return natural is
    variable x : natural := n;
    variable steps : natural := 0;
  begin
    while x /= 1 loop
      if x mod 2 = 0 then x := x / 2; else x := 3 * x + 1; end if;
      steps := steps + 1;
    end loop;
    return steps;
  end function;
  function digits(n : natural) return natural is
    variable x : natural := n;
    variable s : natural := 0;
  begin
    while x > 0 loop s := s + x mod 10; x := x / 10; end loop;
    return s;
  end function;
  function mix(n : natural) return natural is
    function sq(k : natural) return natural is begin return k * k mod 1009; end function;
  begin
    return (sq(n) + digits(n) + collatz(n + 1)) mod 100000;
  end function;
begin
  process
    variable acc : natural := 0;
  begin
    for i in 1 to 150 loop acc := (acc + mix(i)) mod 100000; end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
