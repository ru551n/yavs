-- yavs-test: std=2008
-- expect: E1501 5:23 `e` is not a generic package
-- LRM 4.9
entity e is end entity;
package i is new work.e;
