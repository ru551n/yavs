-- yavs-test: std=2008
-- expect: E1100 13:63 `x` is not declared in `prot_t`
-- LRM 8.3: the suffix must be declared immediately within the protected type (#250)
package p is
  constant x : integer := 1;
  type prot_t is protected
    impure function get return integer;
  end protected;
end package;
package body p is
  type prot_t is protected body
    variable n : integer := 0;
    impure function get return integer is begin return prot_t.x; end function;
  end protected body;
end package body;
