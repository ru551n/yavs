-- yavs-test: std=2008
-- expect: E1104 5:14 `p` is not a context declaration
-- LRM 13.4
package p is end package;
context work.p;
package q is
end package;
