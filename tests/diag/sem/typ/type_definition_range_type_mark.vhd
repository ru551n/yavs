-- yavs-test: std=2008
-- expect: E1231 6:24 the range of a type definition must be `left to right` or a range attribute
-- LRM 5.2.1: the range of a type definition is a range, never a type mark (#822)
package p is
  type small is range -8 to 7;
  type length is range small units
    um;
  end units;
end package;
