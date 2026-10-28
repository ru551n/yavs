package body p is
  function f1 (a : real) return integer is begin return 1; end f2;
  procedure proc1 is begin end procedure proc;
  function "and" (a, b : real) return real is begin return a; end "or";
  function ok (a : real) return real is begin return a; end function OK;
  function "+" (a, b : real) return real is begin return a; end "+";
end package body;
