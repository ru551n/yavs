-- yavs-test: std=2008
-- expect: E1230 7:12 function `twice` has no body
-- LRM 4.2.1, 4.3: a header-instance spec in a package body without its body (#1136)
package q is
end package;
package body q is
  function twice generic (type t) generic map (t => integer) parameter (v : t) return t;
end package body;
