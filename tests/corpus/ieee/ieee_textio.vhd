-- difftest: top=ieee_textio std=2008
-- textio READ/WRITE (and HREAD/HWRITE) of std_logic_vector, fixed and float values through a line.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use ieee.fixed_pkg.all; use ieee.float_pkg.all;
use std.textio.all;
entity ieee_textio is end;
architecture t of ieee_textio is
begin
  process
    variable l : line;
    variable v : std_logic_vector(7 downto 0);
    variable sf : sfixed(3 downto -4);
    variable fl : float32;
    variable ok : boolean;
  begin
    write(l, std_logic_vector'("10ZX01HL"));
    write(l, string'(" "));
    hwrite(l, std_logic_vector'(x"C3"));
    write(l, string'(" "));
    write(l, to_sfixed(-1.25, 3, -4));
    write(l, string'(" "));
    write(l, to_float(0.5));
    report "line [" & l.all & "]";
    read(l, v, ok);
    report "read slv " & to_string(v) & " " & boolean'image(ok);
    assert ok and v = "10ZX01HL" severity failure;
    hread(l, v, ok);
    assert ok and v = x"C3" severity failure;
    read(l, sf, ok);
    report "read sfixed " & to_string(sf) & " " & boolean'image(ok);
    assert ok and to_real(sf) = -1.25 severity failure;
    read(l, fl, ok);
    report "read float " & to_string(fl) & " " & boolean'image(ok);
    assert ok and to_real(fl) = 0.5 severity failure;
    deallocate(l);
    report "done";
    wait;
  end process;
end;
