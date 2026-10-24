-- A call that reported before its loops outgrew the budget finishes lowered (a restart would
-- print the report again); one that had not restarts with its function optimised.
package elab_restart_report_pkg is
  function early (x : integer) return integer;
  function late (x : integer) return integer;
end;
package body elab_restart_report_pkg is
  function early (x : integer) return integer is
    variable a : integer := 0;
  begin
    for i in 1 to x loop
      if i = 3 then report "early report" severity note; end if;
      a := (a + i) mod 10007;
    end loop;
    return a;
  end;
  function late (x : integer) return integer is
    variable a : integer := 0;
  begin
    for i in 1 to x loop
      if i = 4500 then report "late report" severity note; end if;
      a := (a + i) mod 10007;
    end loop;
    return a;
  end;
end;
use work.elab_restart_report_pkg.all;
entity elab_restart_report is end;
architecture a of elab_restart_report is
  constant c1 : integer := early(5000);
  constant c2 : integer := late(5000);
  signal s1 : bit_vector(1 to c1 mod 5 + 1);
  signal s2 : bit_vector(1 to c2 mod 5 + 1);
begin
  process begin
    report "n=" & integer'image(c1) & " " & integer'image(c2);
    wait;
  end process;
end;
