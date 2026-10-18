-- yavs-test: std=2008
-- expect: E1100 5:29 `undefined_thing` is not declared
-- LRM 13.1 (sem.md 8.2: an architecture inherits its entity's failed use clause, no cascade)
package pq is
  constant bad : integer := undefined_thing;
  constant k : integer := 1;
end package;
use work.pq.all;
entity e is end entity;
architecture a of e is
  constant m : integer := k;
begin
end architecture;
