-- yavs-test: std=2019
-- expect: none
-- LRM 6.5.6.2, 6.5.7.2 (difftest generics/generic_types_2019: an entity's generic type and subprogram)
entity swapper is
  generic (type t; function img(x : t) return string);
  port (a, b : in t);
end;
architecture a of swapper is begin
  process begin report img(a) & " " & img(b); wait; end process;
end;
entity generic_types_2019 is end;
architecture t of generic_types_2019 is
  function ii(x : integer) return string is begin return "i" & integer'image(x); end;
  function bb(x : boolean) return string is begin return "b" & boolean'image(x); end;
begin
  u1 : entity work.swapper generic map (t => integer, img => ii) port map (1, 2);
  u2 : entity work.swapper generic map (t => boolean, img => bb) port map (true, false);
end;
