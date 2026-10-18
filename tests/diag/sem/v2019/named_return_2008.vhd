-- yavs-test: std=2008
-- expect: E0004 5:24 a named return value requires VHDL-2019
-- LRM 2019 4.2.1
package p is
  function iota return r of integer_vector;
end package;
