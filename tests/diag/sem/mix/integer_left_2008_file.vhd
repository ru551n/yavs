-- yavs-test: std=2008 flags=--design-std=2019
-- expect: W1008 5:27 `integer'left` means something different under VHDL-2019
-- LRM 2019 0.1
package p is
  constant a : integer := integer'left;
end package;
