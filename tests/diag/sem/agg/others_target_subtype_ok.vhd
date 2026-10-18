-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.3.3
-- An `others` aggregate takes its bounds from the target subtype: a slice
-- (possibly with non-static bounds) or an object whose constraint is not static.
package p is
  function f (w : natural; value : bit_vector; b : boolean) return bit_vector;
end package;
package body p is
  function f (w : natural; value : bit_vector; b : boolean) return bit_vector is
    variable result : bit_vector(7 downto 0) := (others => '0');
    variable pad : bit_vector(1 to value'length + w);
    variable s : string(1 to w);
  begin
    result(w - 1 downto 0) := (others => '1');
    pad := (others => '1');
    pad := (others => '0') when b else (others => '1');
    s := (others => 'c');
    s(1 to 2) := (others => 'd');
    return result;
  end function;
end package body;
