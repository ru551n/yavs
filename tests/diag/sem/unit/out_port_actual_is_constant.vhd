-- yavs-test: std=2008
-- expect: E1360 10:38 `c` is a constant and cannot be an actual for an `out` port
-- LRM 6.5.7.3
entity sub is port (a : out bit); end entity;
architecture r of sub is begin a <= '0'; end architecture;
entity e is end entity;
architecture a of e is
  constant c : bit := '0';
begin
  u : entity work.sub port map (a => c);
end architecture;
