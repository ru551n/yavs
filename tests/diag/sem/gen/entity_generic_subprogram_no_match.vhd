-- yavs-test: std=2019
-- expect: E1300 14:62 no single visible `bb` matches generic subprogram `img`
-- LRM 6.5.7.2
entity swapper is
  generic (type t; function img(x : t) return string);
  port (a : in t);
end;
architecture a of swapper is begin
end;
entity top is end;
architecture t of top is
  function bb(x : boolean) return string is begin return "b"; end;
begin
  u1 : entity work.swapper generic map (t => integer, img => bb) port map (a => 1);
end;
