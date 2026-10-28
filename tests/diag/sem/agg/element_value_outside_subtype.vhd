-- yavs-test: std=2008
-- expect: E1403 13:25 element value 0 is outside subtype `POSITIVE`
-- expect: E1403 14:25 element value 0 is outside subtype `POSITIVE`
-- expect: E1403 15:25 element value 0 is outside subtype `POSITIVE`
-- expect: E1403 16:24 element value 0 is outside subtype `POSITIVE`
-- LRM 9.3.3.3: each element value belongs to the element subtype (#821); like a scalar, an error
-- where the value goes into an object (initial value, assignment, return value)
package p is
  type idx is range 0 to 3;
  type aray is array (idx) of positive;
  type rec is record p : positive; end record;
  constant ok : aray := (others => 1);
  constant c1 : aray := (others => 0);
  constant c2 : aray := (0 => 1, others => 0);
  constant c3 : aray := (1, 2, 0, 4);
  constant r1 : rec := (p => 0);
end package;
