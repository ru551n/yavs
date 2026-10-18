-- yavs-test: std=2008
-- expect: E1002 4:14 context declaration `noctx` not found in library `work`
-- LRM 13.4
context work.noctx;
package q is
end package;
