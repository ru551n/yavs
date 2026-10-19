-- yavs-test: std=2008
-- expect: E1641 8:19 a signal where a label is expected
-- LRM 6.10
entity e is end;
architecture a of e is
  signal s : bit;
  group lbls is (label);
  group g : lbls (s);
begin
  p1 : process begin wait; end process;
end;
