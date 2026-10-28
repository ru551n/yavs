-- yavs-test: std=1993
-- expect: E1007 18:42 port b2 with mode buffer cannot be associated with formal x with mode inout in VHDL-1993
-- expect: E1007 19:51 port io with mode inout cannot be associated with formal y with mode buffer in VHDL-1993
-- LRM 1993 1.1.1.2: a buffer port is associated only with a buffer port, or an in or linkage formal (#814)
entity leaf is
  port (x : inout bit; y : buffer bit; z : in bit);
end entity;
architecture a of leaf is
begin
end architecture;
entity top is
  port (b1, b2 : buffer bit; io : inout bit);
end entity;
architecture a of top is
  signal s1, s2, s3 : bit;
begin
  ok : entity work.leaf port map (x => s1, y => b1, z => b2);
  bad1 : entity work.leaf port map (x => b2, y => s2, z => s1);
  bad2 : entity work.leaf port map (x => s3, y => io, z => s1);
end architecture;
