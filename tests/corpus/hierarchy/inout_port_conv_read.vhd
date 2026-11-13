-- difftest: top=inout_port_conv_read std=2008 ignore=stderr ref=hand
-- An `inout` record port that its instance never drives, read only through a conversion
-- function on the actual of an `in` port inside (a converted `in` port reads the formal, it
-- is no source of it): each element of the formal still contributes its default 'U' to the
-- actual, so the actual stays 'U' although its parent drives every field. A second formal of
-- the same entity type is driven 'Z' inside and resolves to the parent's values.
-- ignore=stderr: the reference simulator warns that the undriven port always contributes
-- 'U', which is the point; the warning is left out of the `.expected`.
-- ref=hand: the first reference simulator's output except one line: through the conversion
-- its leaf of `n` sees the formal's driving value (Z Z); the second reference simulator sees
-- the formal's effective value, its actual's (0 H), as LRM 14.7.3.3 says.
library ieee; use ieee.std_logic_1164.all;
package iopr_pkg is
  type rec_t is record a : std_logic; b : std_logic; end record;
  type oth_t is record x : std_logic; y : std_logic; end record;
  function to_oth (r : rec_t) return oth_t;
end;
package body iopr_pkg is
  function to_oth (r : rec_t) return oth_t is begin return (x => r.a, y => r.b); end;
end;
library ieee; use ieee.std_logic_1164.all; use work.iopr_pkg.all;
entity iopr_leaf is port (o : in oth_t); end;
architecture a of iopr_leaf is begin
  rd : process (o) begin report "leaf x=" & to_string(o.x) & " y=" & to_string(o.y); end process;
end;
library ieee; use ieee.std_logic_1164.all; use work.iopr_pkg.all;
entity iopr_mid is generic (DRIVE : boolean); port (p : inout rec_t); end;
architecture a of iopr_mid is begin
  l : entity work.iopr_leaf port map (o => to_oth(p));
  g : if DRIVE generate
    drv : p <= ('Z', 'Z');
  end generate;
end;
library ieee; use ieee.std_logic_1164.all; use work.iopr_pkg.all;
entity inout_port_conv_read is end;
architecture a of inout_port_conv_read is
  signal s, t : rec_t;
begin
  m : entity work.iopr_mid generic map (false) port map (p => s);
  n : entity work.iopr_mid generic map (true) port map (p => t);
  drv_s : s <= ('1', '0');
  drv_t : t <= ('0', 'H');
  rd : process (s, t) begin
    report "top s.a=" & to_string(s.a) & " s.b=" & to_string(s.b) & " t.a=" & to_string(t.a)
      & " t.b=" & to_string(t.b);
  end process;
  check : process begin
    wait for 1 ns;
    assert s = ('U', 'U') report "undriven inout port lost its 'U'" severity failure;
    assert t = ('0', 'H') report "t" severity failure;
    report "done";
    wait;
  end process;
end;
