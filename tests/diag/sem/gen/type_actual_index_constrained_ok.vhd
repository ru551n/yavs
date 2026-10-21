-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.2 (UVVM predefined_sb: a generic type actual with an index constraint; sim1 and sim3 accept)
library ieee; use ieee.std_logic_1164.all;
package g is
  generic (type t_element; function element_match (a, b : t_element) return boolean);
  function m (a, b : t_element) return boolean;
end package;
package body g is
  function m (a, b : t_element) return boolean is
  begin
    return element_match(a, b);
  end function;
end package body;
library ieee; use ieee.std_logic_1164.all, ieee.numeric_std.all;
package c is
  constant C_W : natural := 8;
end package;
library ieee; use ieee.std_logic_1164.all, ieee.numeric_std.all;
use work.c.all;
package slv_sb is new work.g
  generic map (t_element => std_logic_vector(C_W - 1 downto 0), element_match => std_match);
