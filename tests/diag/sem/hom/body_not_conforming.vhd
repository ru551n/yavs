-- yavs-test: std=2008
-- expect: E1240 14:3 the body of `p1` does not conform to its declaration
-- expect: E1240 17:3 the body of `p2` does not conform to its declaration
-- expect: E1240 20:3 the body of `p3` does not conform to its declaration
-- LRM 4.10: a subprogram body's specification is the declaration's lexical elements, apart from literal spelling and expanded names (#808)
package p is
  procedure p1 (x, y : integer);
  procedure p2 (x : integer; y : integer);
  procedure p3 (x : integer);
  function f1 (x : integer := 10) return integer;
  function f2 (x : integer) return integer;
end package;
package body p is
  procedure p1 (x : integer; y : integer) is
  begin
  end procedure;
  procedure p2 (x, y : in integer) is
  begin
  end procedure;
  procedure p3 (constant x : integer) is
  begin
  end procedure;
  function f1 (X : INTEGER := 1_0) return integer is
  begin
    return x;
  end function;
  function f2 (x : std.standard.integer) return integer is
  begin
    return x;
  end function;
end package body;
