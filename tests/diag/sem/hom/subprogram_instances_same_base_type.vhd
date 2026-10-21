-- yavs-test: std=2019
-- expect: E1101 9:13 homograph of the std_ulogic instance (std_logic has the same base type)
-- LRM 12.3, 4.5.1: two instances whose generic actuals have the same base type are homographs (#179; sim1 agrees, sim3 accepts)
library ieee;
use ieee.std_logic_1164.all;
package p is
  procedure gc generic (type t) parameter (signal clock : inout t);
  procedure gc is new gc generic map (t => std_ulogic);
  procedure gc is new gc generic map (t => std_logic);
  procedure gc is new gc generic map (t => bit);
end package;
