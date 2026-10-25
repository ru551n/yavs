-- A process without a wait statement stays in compiled code across its jumps back (#583): 2.5
-- million of them, a few exits for the checks, none per jump.
use std.env.all;
entity llvm_nowait is end;
architecture a of llvm_nowait is
  signal a : bit := '0';
begin
  process
    variable n : natural := 0;
  begin
    a <= not a;
    n := n + 1;
    if n = 2500000 then
      report "jumps=" & integer'image(n);
      finish;
    end if;
  end process;
end;
