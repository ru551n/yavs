-- yavs-test: std=2008
-- expect: E1234 7:23 a subprogram interface declaration is not a port
-- help: generic list
-- LRM 6.5.6.3: a port clause declares interface signals only
package p is
  component c is
    port (s : in bit; function f return integer);
  end component;
end package;
