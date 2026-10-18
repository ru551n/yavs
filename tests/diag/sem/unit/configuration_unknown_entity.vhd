-- yavs-test: std=2008
-- expect: E1002 4:22 entity `ghost` not found in library `work`
-- LRM 3.4
configuration cfg of ghost is
  for rtl
  end for;
end configuration;
