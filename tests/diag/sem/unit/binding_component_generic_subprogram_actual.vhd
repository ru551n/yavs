-- yavs-test: std=2019
-- expect: E1506 17:57 a component's generic subprogram as the actual in a binding's generic map
-- LRM 7.3.3, 6.5.7.2 (user decision, 2026-10-19, L63)
-- The reference simulators reject it; a default binding (by name) stays legal.
entity sub is
  generic (function f(x : integer) return integer);
  port (q : out integer);
end entity;
architecture r of sub is begin q <= f(1); end architecture;
entity e is end entity;
architecture a of e is
  component sub is
    generic (function f(x : integer) return integer);
    port (q : out integer);
  end component;
  function g(x : integer) return integer is begin return x; end;
  for u1 : sub use entity work.sub(r) generic map (f => f);
  for u2 : sub use entity work.sub(r) generic map (f => g);
  signal s1, s2, s3 : integer;
begin
  u1 : sub generic map (f => g) port map (s1);
  u2 : sub generic map (f => g) port map (s2);
end architecture;
