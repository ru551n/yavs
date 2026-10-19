-- A shared variable of a protected type declared in the package declaration, before the
-- protected body (which is in the package body).
package p is
  type pt is protected
    impure function get return natural;
  end protected;
  shared variable sv : pt;
end package;
package body p is
  type pt is protected body
    impure function get return natural is begin return 1; end;
  end protected body;
end package body;
