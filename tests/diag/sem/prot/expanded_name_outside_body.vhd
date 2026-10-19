-- yavs-test: std=2008
-- expect: E1103 18:17 `n` exists but is not visible here
-- expect: E1103 19:17 `get` exists but is not visible here
-- help: `n` is declared in protected type `prot_t` and is only visible inside it
-- LRM 8.3: an expanded name with a protected type prefix is allowed only inside the type (#250)
package p is
  type prot_t is protected
    impure function get return integer;
  end protected;
end package;
package body p is
  type prot_t is protected body
    variable n : integer := 0;
    impure function get return integer is begin return n; end function;
  end protected body;
  procedure q (x : out integer) is
  begin
    x := prot_t.n;
    x := prot_t.get;
  end procedure;
end package body;
