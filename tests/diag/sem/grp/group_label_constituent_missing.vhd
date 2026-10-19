-- yavs-test: std=2008
-- expect: E1100 7:23 no statement has this label
-- LRM 6.10
entity e is end;
architecture a of e is
  group lbls is (label, label);
  group g : lbls (p1, nolabel);
begin
  p1 : process begin wait; end process;
end;
