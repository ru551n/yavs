-- yavs-test: std=2008
-- expect: E1905 8:26 a record element cannot be of a protected type in 2008
-- LRM 5.6.1 (2008): record elements of protected types arrive with VHDL-2019 (LCS-2016-014)
package p is
  type pt is protected
    procedure inc;
  end protected;
  type rec is record a : pt; end record;
end package;
