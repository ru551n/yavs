-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.2, 4.9 (mode views declared in a generic package belong to each instance)
package gp is
  generic (type data_t);
  type bus_t is record d : data_t; v : bit; ack : bit; end record;
  view src of bus_t is d, v : out; ack : in; end view;
  alias snk is src'converse;
  procedure drive (signal b : view src; x : data_t);
end package;
package body gp is
  procedure drive (signal b : view src; x : data_t) is
  begin
    b.d <= x;
    b.v <= '1';
  end procedure;
end package body;
package ip is new work.gp generic map (data_t => integer);
use work.ip.all;
entity m is port (b : view src); end entity;
architecture a of m is
begin
  process begin drive(b, 5); wait; end process;
end architecture;
use work.ip.all;
entity s is port (b : view snk); end entity;
architecture a of s is
begin
  b.ack <= b.v;
end architecture;
