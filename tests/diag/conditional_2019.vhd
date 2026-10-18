-- yavs: --std 2019
-- `( conditional_expression )` is a primary; a qualified expression needs its own parentheses.
package p is
  constant g : boolean := true;
  constant a : integer := 3 + (1 when g else 2);
  constant b : integer := integer'(1 when g else 2);
end package;
