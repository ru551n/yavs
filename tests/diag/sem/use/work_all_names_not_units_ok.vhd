-- yavs-test: std=2008
-- expect: none
-- LRM 12.4: `use work.all` makes the primary units of the library potentially visible; a name
-- that is no unit of it (`integer`, an operator) is looked up elsewhere, not an error.
use work.all;
entity e is
end entity;
architecture a of e is
  function weird (bw : integer range 2 to 32) return integer is
  begin
    return -(2 ** (bw - 1));
  end function;
begin
end architecture;
