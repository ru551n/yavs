-- yavs-test: std=2008
-- expect: E1502 15:9 generic package `p` has no actual
-- A formal package with a generic map still needs an actual instance: it has no default and
-- is not instantiated from its own map (LRM 6.5.7.2; L132, the choice differs from one reference).
package slv is
  generic (n : integer);
  constant w : integer := n;
end package;

package wrap is
  generic (package p is new work.slv generic map (n => 4));
  constant c : integer := p.w;
end package;

package inst is new work.wrap;
