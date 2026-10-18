-- yavs-test: std=2008
-- expect: E0004 5:12 a generic type definition requires VHDL-2019
-- LRM 2019 0.1
package g is
  generic (type t is (<>));
end package;
