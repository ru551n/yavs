-- yavs-test: std=2008 flags=--design-std=2019
-- expect: W1008 5:27 `natural'high` means something different under VHDL-2019
-- LRM 2019 0.1
package p is
  constant m : integer := natural'high;
end package;
