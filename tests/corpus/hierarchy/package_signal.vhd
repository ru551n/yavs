-- difftest: top=package_signal std=2008
-- Package signals (#183): one read only, one initialised by a concatenation of function
-- results (VUnit run_pkg's runner) and driven by a process and through a signal parameter.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package package_signal_ev is
  subtype ev_t is std_logic_vector(0 to 3);
  impure function new_ev(n : natural) return ev_t;
  procedure set(signal s : inout std_logic_vector; v : std_logic_vector);
end package;
package body package_signal_ev is
  impure function new_ev(n : natural) return ev_t is
  begin return std_logic_vector(to_unsigned(n, 4)); end;
  procedure set(signal s : inout std_logic_vector; v : std_logic_vector) is
  begin s <= v; end;
end package body;
library ieee; use ieee.std_logic_1164.all; use work.package_signal_ev.all;
package package_signal_q is
  signal ro : std_logic_vector(0 to 3) := "0101";
  signal s : std_logic_vector(0 to 8) := new_ev(1) & new_ev(2) & '0';
end package;
library ieee; use ieee.std_logic_1164.all; use work.package_signal_q.all; use work.package_signal_ev.all;
entity package_signal is end;
architecture t of package_signal is begin
  process begin
    report "ro=" & to_string(ro) & " s=" & to_string(s);
    s <= (others => '0');
    wait for 1 ns;
    report "s=" & to_string(s);
    set(s, "111100001");
    wait for 1 ns;
    report "s=" & to_string(s);
    wait;
  end process;
  process (s) begin report "seen " & to_string(s); end process;
end;
