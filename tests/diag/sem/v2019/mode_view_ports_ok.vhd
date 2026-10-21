-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.2 (OSVVM Axi4InterfaceModeViewPkg: views, alias of 'converse, view ports; sim1 and sim3 accept)
library ieee; use ieee.std_logic_1164.all;
package bus_pkg is
  type bus_t is record
    req  : std_logic;
    ack  : std_logic;
    data : std_logic_vector(7 downto 0);
  end record;
  view manager of bus_t is
    req, data : out;
    ack : in;
  end view;
  alias subordinate is manager'converse;
end package;
library ieee; use ieee.std_logic_1164.all;
use work.bus_pkg.all;
entity m is
  port (b : view manager of bus_t);
end entity;
architecture a of m is
begin
  b.req <= '1';
  b.data <= (others => '0');
end architecture;
library ieee; use ieee.std_logic_1164.all;
use work.bus_pkg.all;
entity s is
  port (b : view subordinate);
end entity;
architecture a of s is
begin
  b.ack <= b.req;
end architecture;
library ieee; use ieee.std_logic_1164.all;
use work.bus_pkg.all;
entity c is
  port (b : view manager'converse of bus_t);
end entity;
architecture a of c is
begin
  b.ack <= '0';
end architecture;
