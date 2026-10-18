-- yavs-test: std=2008
-- expect: none
-- LRM 12.3: an explicit declaration in the body hides the implicit operation of a package type
package p is
  type arr is array (natural range <>) of integer;
end package;
package body p is
  function "=" (l, r : arr) return boolean is
  begin
    return true;
  end function;
end package body;
