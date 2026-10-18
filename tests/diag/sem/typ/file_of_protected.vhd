-- yavs-test: std=2008
-- expect: E1222 6:21 a file cannot contain protected type `pt`
-- LRM 5.5
package p is
  type pt is protected end protected;
  type f is file of pt;
end package;
