-- yavs-test: std=2019
-- expect: E1903 14:5 port `b` has mode `in` and cannot be driven
-- LRM 2019 6.5.2 (the instance's view keeps the element modes)
package gp is
  generic (type data_t);
  type bus_t is record d : data_t; ack : bit; end record;
  view src of bus_t is d : out; ack : in; end view;
end package;
package ip is new work.gp generic map (data_t => integer);
use work.ip.all;
entity m is port (b : view src); end entity;
architecture a of m is
begin
    b.ack <= '1';
end architecture;
