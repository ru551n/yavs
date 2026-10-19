-- yavs-test: std=2008
-- expect: E1234 10:9 a package interface declaration is not a port
-- help: generic list
-- LRM 6.5.6.3: a port clause declares interface signals only
package g is
  generic (n : integer);
  constant c : integer := n;
end package;
entity e is
  port (package q is new work.g generic map (<>));
end entity;
