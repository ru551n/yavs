-- std.env 2019: what GetVhdlAssertFormat returns after SetVhdlAssertFormat. The LRM does not
-- say; yavs returns the string as set, the other simulator a normalised form.
use std.env.all;
entity top is end entity;
architecture a of top is
begin
  process
    variable valid : boolean;
  begin
    SetVhdlAssertFormat(error, "{t} x {r:#<10}", valid);
    report "RES valid=" & boolean'image(valid);
    report "RES get=" & GetVhdlAssertFormat(error);
    wait;
  end process;
end architecture;
