-- LRM 14.4.2: the package declaration is elaborated before its body, so a constant of the
-- declaration cannot call a function of the same package (#197). Only a process reads it.
package body_not_elaborated_pkg is
  function f return integer;
  constant c : integer := f;
end package;

package body body_not_elaborated_pkg is
  function f return integer is
  begin
    return 1;
  end function;
end package body;

use work.body_not_elaborated_pkg.all;

entity body_not_elaborated is
end entity;

architecture a of body_not_elaborated is
begin
  process
  begin
    report integer'image(c);
    wait;
  end process;
end architecture;
