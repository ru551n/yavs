-- yavs-test: std=2019
-- expect: none
-- LRM 2019 4.2.2.3, 6.5.2 (OSVVM AddressBusTransactionPkg: signal parameters with mode views; sim1 and sim3 accept)
library ieee; use ieee.std_logic_1164.all;
package bus_pkg is
  type bus_t is record
    req  : std_logic;
    ack  : std_logic;
  end record;
  view manager of bus_t is
    req : out;
    ack : in;
  end view;
  procedure handshake (signal b : view manager of bus_t);
end package;
package body bus_pkg is
  procedure handshake (signal b : view manager of bus_t) is
  begin
    b.req <= '1';
    wait until b.ack = '1';
    b.req <= '0';
  end procedure;
end package body;
library ieee; use ieee.std_logic_1164.all;
use work.bus_pkg.all;
entity t is end entity;
architecture a of t is
  signal b : bus_t;
begin
  process
  begin
    handshake(b);
    wait;
  end process;
  b.ack <= b.req;
end architecture;
