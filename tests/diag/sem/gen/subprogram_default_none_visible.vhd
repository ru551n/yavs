-- yavs-test: std=2008
-- expect: E1504 7:43 no visible `"+"` for the `is <>` default of `+`
-- LRM 6.5.6.3
package g is
  generic (type t; function "+" (a, b : t) return t is <>);
end package;
package i is new work.g generic map (t => bit);
