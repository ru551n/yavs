-- yavs-test: std=2008
-- expect: E1004 5:19 `p` is a package, not an entity
-- LRM 3.3
package p is end package;
architecture a of p is
begin
end architecture;
