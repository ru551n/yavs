-- yavs-test: std=2008
-- expect: E1301 11:12 no operator `+` matches the operands of type `t`
-- LRM 6.5.3
package g is
  generic (type t);
  function f (a, b : t) return t;
end package;
package body g is
  function f (a, b : t) return t is
  begin
    return a + b;
  end function;
end package body;
