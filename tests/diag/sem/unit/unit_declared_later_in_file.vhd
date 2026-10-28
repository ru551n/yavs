-- yavs-test: std=2008
-- expect: E1002 4:10 design unit `later_pkg` of library `work` is declared later in this file
-- LRM 13.5: the units of a file are analyzed in textual order; a unit cannot need a later one (#810)
use work.later_pkg.all;
entity e is
end entity;
architecture a of e is
  signal s : later_t;
begin
end architecture;
package later_pkg is
  type later_t is (x, y);
end package;
