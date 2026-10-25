-- A function and a procedure that read signals (a logic one and a real one) and wait for nothing run
-- on native frames: their calls from compiled code are direct calls, not steps of the interpreter.
library ieee;
use ieee.std_logic_1164.all;
entity llvm_sigfun is end;
architecture a of llvm_sigfun is
  signal en  : std_logic := '1';
  signal rt  : real := 0.5;
  signal acc : integer := 0;
  impure function gate(k : integer) return integer is
  begin
    if en = '1' then return k + 1; else return k; end if;
  end function;
  procedure scale(variable v : inout integer) is
  begin
    if rt > 0.25 then v := (v * 3) mod 1009; end if;
  end procedure;
begin
  process
    variable v : integer := 1;
    procedure step is
    begin
      v := gate(v);
      scale(v);
    end procedure;
  begin
    for i in 1 to 300 loop
      step;
    end loop;
    acc <= v;
    wait for 1 ns;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
