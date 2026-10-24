-- A pure function whose one elaboration-time call runs a long loop: the call stops after a
-- budget of jumps back, the function is optimised, and the call runs again (`elab_restarts`).
package elab_restart_pkg is
  function big (x : integer) return integer;
end;
package body elab_restart_pkg is
  function big (x : integer) return integer is
    variable a : integer := 0;
  begin
    for i in 1 to x loop a := (a + i * 3) mod 10007; end loop;
    return a;
  end;
end;
use work.elab_restart_pkg.all;
entity elab_restart is end;
architecture a of elab_restart is
  constant c : integer := big(9000);
  signal s : bit_vector(1 to c mod 7 + 1);
begin
  process begin
    report "n=" & integer'image(c) & " " & integer'image(s'length);
    wait;
  end process;
end;
