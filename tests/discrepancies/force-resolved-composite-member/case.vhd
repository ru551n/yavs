-- LRM 10.5.2.1: the target of a force is a member of a record signal resolved as a whole.
package pkg is
  type rec_u is record a : integer; n : integer; end record;
  type rec_drv is array (natural range <>) of rec_u;
  function first(v : rec_drv) return rec_u;
  subtype rec_t is first rec_u;
end package;
package body pkg is
  function first(v : rec_drv) return rec_u is
  begin
    return v(v'low);
  end function;
end package body;
use work.pkg.all;
entity top is end entity;
architecture a of top is
  signal s : rec_t := (0, 0);
begin
  process begin
    s.n <= force 40;
    wait for 1 ns;
    report "RES forced " & integer'image(s.n);
    wait;
  end process;
end architecture;
