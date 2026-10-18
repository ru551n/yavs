-- yavs-test: std=2008
-- expect: E1002 20:10 design unit `nonexistent` not found in library `work`
-- LRM 13.1 (sem.md 8.2: a generic subprogram actual after a failed type actual is a cascade; OSVVM ScoreboardPkg_Uart)
package g2 is
  generic (type et; impure function ets (a : et) return string);
end package;
library ieee; use ieee.std_logic_1164.all;
package tb is
  type stim is record data : std_logic_vector(7 downto 0); end record;
  function to_string (constant rec : in stim) return string;
end package;
package body tb is
  function to_string (constant rec : in stim) return string is
  begin
    return to_string(rec.data);
  end function;
end package body;
use work.tb.all;
package inst is new work.g2 generic map (et => stim, ets => to_string);
use work.nonexistent.all;
package inst2 is new work.g2 generic map (et => stim, ets => to_string);
