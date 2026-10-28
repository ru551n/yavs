-- yavs-test: std=2008
-- expect: E1100 9:36 `widht` is not declared in `pkg`
-- help: a constant with a similar name exists: `width`
-- LRM 12.3
package pkg is
  constant width : integer := 4;
end package;
package q is
  constant w : integer := work.pkg.widht;
end package;
