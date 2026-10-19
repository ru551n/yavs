-- LRM 6.5.3, 6.5.7.2: does a positional generic map count the implicit "=" and "/=" that follow
-- an interface type?
entity sub is
  generic (type t; width : natural);
end entity;
architecture a of sub is
begin
  process begin report "RES width=" & integer'image(width); wait; end process;
end architecture;
entity top is end entity;
architecture a of top is
begin
  u : entity work.sub generic map (integer, 8);
end architecture;
