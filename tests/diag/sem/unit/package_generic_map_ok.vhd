-- yavs-test: std=2008
-- expect: none
-- LRM 4.7, 4.9: a package declaration with a generic map aspect is used like an instance
package gm is
  generic (g : integer := 1; type t);
  generic map (g => 2, t => bit);
  constant c : integer := g;
  type arr is array (0 to g) of t;
end package;
use work.gm.all;
entity e is end;
architecture a of e is
  constant z : arr := (others => '1');
  package q is
    generic (h : integer := 5);
    generic map (h => c + 1);
    constant d : integer := h * 2;
  end package;
  constant k : integer := q.d + c;
begin
end architecture;
