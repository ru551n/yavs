-- Three files that depend on each other, analysed one file per call (unit_mid.vhd, unit_tb.vhd).
library ieee; use ieee.std_logic_1164.all;
package unit_pkg is
  constant K : integer := 7;
  function f (x : std_logic_vector) return integer;
end;
package body unit_pkg is
  function f (x : std_logic_vector) return integer is begin return x'length * K; end;
end;
