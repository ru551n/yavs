-- yavs-test: std=2008
-- expect: none
-- LRM 12.4 (VUnit com_common: use of a type makes its literals visible; sim1 and sim2 accept)
package tp is
  type st_t is (ok, timeout, bad);
  type len_t is range 0 to 100000 units u; ku = 1000 u; end units;
end package;
use work.tp.st_t;
use work.tp.len_t;
package q is
  function f (s : st_t) return boolean;
  constant k : len_t := 2 ku;
end package;
package body q is
  function f (s : st_t) return boolean is
  begin
    return s = ok or s > timeout;
  end function;
end package body;
