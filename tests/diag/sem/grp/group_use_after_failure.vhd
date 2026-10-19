-- yavs-test: std=2008
-- expect: E1641 6:3 template takes 2 constituents
-- LRM 6.10
package p is
  group two is (signal, signal);
  group g : two (nothing_here_is_not_reported_after_arity);
  attribute tag : integer;
  attribute tag of g : group is 1;
end package;
