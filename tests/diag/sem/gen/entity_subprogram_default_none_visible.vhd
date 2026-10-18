-- yavs-test: std=2019
-- expect: E1504 10:3 no visible `img` for the `is <>` default of `img`
-- LRM 6.5.6.2
entity d is
  generic (type t; v : t; function img(x : t) return string is <>);
end entity;
architecture a of d is begin end architecture;
entity e is end entity;
architecture a of e is begin
  u : entity work.d generic map (t => integer, v => 4);
end architecture;
