-- yavs-test: std=2019
-- expect: none
-- LRM 6.5.6 (2019), LCS-2016-086: interface lists can refer to earlier elements; an entity may declare a subprogram whose body is in an architecture not analyzed here (#177)
entity e086 is
  generic (g1 : integer; g2 : integer := g1; g3, g4 : integer);
  port (p1 : string(g3 to g4); p2 : string(p1'range); p3 : p1'subtype);
  procedure x (y1, y2 : integer; y3 : integer range y1 to y2; y4 : y1'subtype);
end e086;
