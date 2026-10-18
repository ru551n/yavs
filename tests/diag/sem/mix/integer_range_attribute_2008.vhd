-- yavs-test: std=2008 flags=--design-std=2019
-- expect: W1008 5:30 `integer'low` means something different under VHDL-2019
-- LRM 2019 0.1
package p is
  subtype w is integer range integer'low to 0;
end package;
