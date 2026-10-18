-- yavs-test: std=2008
-- expect: E1232 6:25 a type interface in a parameter list
-- help: generic list
-- LRM 4.2.2.1: a parameter is an interface constant, signal, variable or file
package p is
  function f parameter (type t; v : integer) return integer;
end package;
