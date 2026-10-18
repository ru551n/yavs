-- yavs-test: std=2008
-- expect: E1232 6:31 a subprogram interface in a parameter list
-- help: generic list
-- LRM 4.2.2.1: a parameter is an interface constant, signal, variable or file
package p is
  procedure run (v : integer; function g (x : integer) return integer);
end package;
