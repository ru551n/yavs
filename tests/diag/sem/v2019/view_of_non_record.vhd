-- yavs-test: std=2019
-- expect: E1902 5:13 `integer` is not a record type
-- LRM 2019 6.5.2
package p is
  view v of integer is
  end view;
end package;
