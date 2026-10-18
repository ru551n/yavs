-- yavs-test: std=2008
-- expect: W1501 5:12 generic `n` is never used
-- LRM 6.5.6
package g is
  generic (n : integer := 1);
  constant w : integer := 4;
end package;
