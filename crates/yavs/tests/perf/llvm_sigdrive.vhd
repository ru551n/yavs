-- Procedures that drive signals (one through a signal parameter, one nested in the process) and a
-- function that reads a vector signal run on native frames: their calls from compiled code are
-- direct calls, not steps of the interpreter, and the assignments are the kernel's fast answer.
library ieee;
use ieee.std_logic_1164.all;
entity llvm_sigdrive is end;
architecture a of llvm_sigdrive is
  signal q    : std_logic := '0';
  signal v    : std_logic_vector(7 downto 0) := x"a5";
  signal cnt  : integer := 0;
  signal acc  : integer := 0;
  procedure put (signal s : out std_logic; signal c : out integer; x : integer) is
  begin
    if x mod 2 = 0 then s <= '1'; else s <= '0'; end if;
    c <= x;
  end procedure;
  impure function ones return integer is
    variable k : integer := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then k := k + 1; end if;
    end loop;
    return k;
  end function;
begin
  process
    variable t : integer := 0;
    procedure bump (x : integer) is
    begin
      acc <= x + t;
    end procedure;
  begin
    for i in 1 to 300 loop
      put(q, cnt, i);
      t := t + ones;
      bump(i);
      wait for 1 ns;
    end loop;
    report "acc=" & integer'image(acc) & " t=" & integer'image(t);
    wait;
  end process;
end;
