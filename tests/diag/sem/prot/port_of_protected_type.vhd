-- yavs-test: std=2008
-- expect: E1905 11:9 a port cannot have protected type `pt`
-- LRM 5.6.2
package p is
  type pt is protected
    procedure set (x : integer);
  end protected;
end package;
use work.p.all;
entity e is
  port (x : in pt);
end entity;
