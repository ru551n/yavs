-- Subprograms that read the name of an item of a library package do not depend on the design they
-- are lowered for (the name is a constant, #460).
package ctx is
  constant c : integer := 1;
  function who return string;
  function wrap return string;
  function plain return integer;
end package;

package body ctx is
  function who return string is
  begin
    return c'instance_name;
  end function;
  function wrap return string is
  begin
    return who;
  end function;
  function plain return integer is
  begin
    return c + 2;
  end function;
end package body;
