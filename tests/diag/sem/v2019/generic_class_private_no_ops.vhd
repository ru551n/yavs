-- yavs-test: std=2019
-- expect: E1301 11:12 no operator `<` matches the operands of type `t`
-- LRM 2019 6.5.3
package g is
  generic (type t is private);
  function same (a, b : t) return boolean;
end package;
package body g is
  function same (a, b : t) return boolean is
  begin
    return a < b;
  end function;
end package body;
