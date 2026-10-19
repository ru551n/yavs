-- std.reflection (LRM 2019 16.11): simple_name of the mirror of a predefined type. The
-- declared identifier is spelled INTEGER in std.standard; the case of the returned string is
-- not stated.
use std.reflection.all;
entity reflname is end entity;

architecture test of reflname is
begin
  process
    variable m : subtype_mirror;
  begin
    m := integer'reflect;
    report "RES simple " & m.simple_name;
    wait;
  end process;
end architecture;
