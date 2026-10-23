-- difftest: top=predefined_names std=1993
-- Names that are predefined or reserved in VHDL-2008 are free in VHDL-93: a package may declare
-- its own TO_STRING, MAXIMUM and INTEGER_VECTOR, and words such as default are identifiers.
package user is
  type integer_vector is array (natural range <>) of integer;
  type boolean_vector is array (natural range <>) of boolean;
  function to_string (x : integer) return string;
  function maximum (a, b : integer) return integer;
  function minimum (a, b : integer) return integer;
  constant default : integer := 7;
  constant flush : integer := 3;
end package;

package body user is
  function to_string (x : integer) return string is
  begin
    return "<" & integer'image(x) & ">";
  end;
  function maximum (a, b : integer) return integer is
  begin
    if a > b then return a; else return b; end if;
  end;
  function minimum (a, b : integer) return integer is
  begin
    if a < b then return a; else return b; end if;
  end;
end package body;

use work.user.all;

entity predefined_names is end;

architecture a of predefined_names is
  signal context, force, release, parameter, property : integer := 1;
  signal sequence, cover, assume, protected, strong : integer := 2;
  signal restrict, fairness, vunit, vmode, vprop : integer := 3;
  signal v : integer_vector(1 to 3) := (4, 5, 6);
  signal b : boolean_vector(1 to 2) := (true, false);
begin
  process
  begin
    report to_string(maximum(3, minimum(9, 4))) & " " & integer'image(default + flush);
    report integer'image(context + force + release + parameter + property
                         + sequence + cover + assume + protected + strong
                         + restrict + fairness + vunit + vmode + vprop);
    assert v(2) = 5 and b(1) report "vectors" severity failure;
    wait;
  end process;
end architecture;
