-- math_real.uniform in compiled code is computed in place (no callback per call), for the seeds
-- and the real in the variables of the process, reached through a nested procedure and through
-- by-reference parameters of an architecture procedure.
library ieee;
use ieee.math_real.all;
entity llvm_uniform is end;
architecture a of llvm_uniform is
  procedure draw(variable a, b : inout positive; variable x : inout real) is
  begin
    uniform(a, b, x);
  end procedure;
begin
  process
    variable s1 : positive := 123456;
    variable s2 : positive := 987654;
    variable x  : real := 0.0;
    variable acc : real := 0.0;
    procedure local is
    begin
      uniform(s1, s2, x);
      acc := acc + x;
    end procedure;
  begin
    for i in 1 to 200 loop
      local;
      draw(s1, s2, x);
      acc := acc + x;
    end loop;
    report "acc=" & real'image(acc) & " s1=" & integer'image(s1) & " s2=" & integer'image(s2);
    wait;
  end process;
end;
