-- difftest: top=file_in_package std=2008
-- Files declared in packages with an open clause are opened when the package is elaborated
-- (#834), in two packages, used from a process and from a package subprogram.
use std.textio.all;
package fip_a is
  file g : text open write_mode is "fip_g.txt";
end package;

use std.textio.all;
package fip_b is
  file f : text open write_mode is "fip_f.txt";
  procedure put (s : string);
end package;

package body fip_b is
  procedure put (s : string) is
    variable l : line;
  begin
    write(l, s);
    writeline(f, l);
  end procedure;
end package body;

use std.textio.all;
use work.fip_a.all;
use work.fip_b.all;
entity file_in_package is end;
architecture t of file_in_package is
begin
  process
    variable l : line;
  begin
    put("from b");
    write(l, string'("from a"));
    writeline(g, l);
    file_close(f);
    file_close(g);
    file_open(f, "fip_g.txt", read_mode);
    readline(f, l);
    report "g holds " & l.all;
    file_close(f);
    file_open(g, "fip_f.txt", read_mode);
    readline(g, l);
    report "f holds " & l.all;
    assert endfile(g);
    file_close(g);
    wait;
  end process;
end;
