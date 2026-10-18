-- rsim-test: std=2008
-- expect: E1201 6:14 `e` denotes an entity, not a type
-- LRM 6.3
entity e is end entity;
package p is
  signal s : e;
end package;
