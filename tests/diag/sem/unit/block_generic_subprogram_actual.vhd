-- yavs-test: std=2019
-- expect: E1506 19:44 a block's generic subprogram as the actual of an instance's generic subprogram
-- LRM 6.5.7.2 (user decision, 2026-10-19, L63)
-- Same for a block: the reference simulators reject it; a local function is legal.
entity sub is
  generic (function f(x : integer) return integer);
  port (q : out integer);
end entity;
architecture r of sub is begin q <= f(1); end architecture;
entity e is end entity;
architecture a of e is
  function g(x : integer) return integer is begin return x; end;
  signal s1, s2 : integer;
begin
  b : block
    generic (function bf(x : integer) return integer);
    generic map (bf => g);
  begin
    u1 : entity work.sub generic map (f => bf) port map (s1);
    u2 : entity work.sub generic map (f => g) port map (s2);
  end block;
end architecture;
