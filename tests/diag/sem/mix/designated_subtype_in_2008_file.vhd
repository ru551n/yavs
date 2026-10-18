-- yavs-test: std=2008
-- expect: E1007 6:19 `'designated_subtype` is new in VHDL-2019
-- LRM 16.2.2 (2019)
package p is
  type ip is access integer;
  subtype s is ip'designated_subtype;
end package;
