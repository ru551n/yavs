-- LRM 6.5.2, 14.7.2: a signal parameter with a mode view drives only its out, inout and
-- buffer elements. Here the formal is associated element by element and its `in` element
-- ready has a second source (the testbench process), which is legal only if the call does
-- not drive it.
library ieee;
use ieee.std_logic_1164.all;
package pkg is
  type bus_t is record valid : bit; data : integer; ready : std_logic; end record;
  view bus_m of bus_t is valid, data : out; ready : in; end view;
  procedure send (signal b : view bus_m; d : integer);
end package;
package body pkg is
  procedure send (signal b : view bus_m; d : integer) is
  begin
    b.data <= d;
    b.valid <= '1';
    wait for 2 ns;
    report "RES ready=" & std_logic'image(b.ready);
  end procedure;
end package body;
library ieee;
use ieee.std_logic_1164.all;
use work.pkg.all;
entity top is end entity;
architecture a of top is
  signal v : bit;
  signal d : integer;
  signal r : std_logic;
begin
  process begin
    send(b.valid => v, b.data => d, b.ready => r, d => 3);
    wait;
  end process;
  process begin
    r <= '1';
    wait;
  end process;
end architecture;
