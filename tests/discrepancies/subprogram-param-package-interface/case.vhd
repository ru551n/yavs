-- yavs-test: std=2008
-- expect: E1232 10:15 a package interface in a parameter list
-- help: generic list
-- LRM 4.2.2.1: a parameter is an interface constant, signal, variable or file
package g is
  generic (n : integer);
  constant c : integer := n;
end package;
package p is
  function f (package q is new work.g generic map (<>)) return integer;
end package;
