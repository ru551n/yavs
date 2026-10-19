-- yavs-test: std=2008
-- expect: none
-- LRM 6.10
entity e is end;
architecture a of e is
  group lbls is (label, label <>);
  group g : lbls (p1, b1);
  attribute tag : integer;
  attribute tag of g : group is 1;
begin
  p1 : process begin wait; end process;
  b1 : block begin end block;
end;
