-- yavs-test: std=2019
-- expect: E1630 20:40 no formal `f` has profile [real return real]
-- expect: E1630 22:39 `n` is not a subprogram
-- expect: E1630 23:47 no formal `g` has profile [bit return bit]
-- LCS-2016-I03: a formal signature that matches no formal of that name
entity sub is
  generic (n : integer; function g (x : integer) return integer);
end entity;
architecture a of sub is
begin
end architecture;
entity e is end entity;
architecture a of e is
  function inc (x : integer) return integer is begin return x + 1; end function;
  function ap generic (function f (x : integer) return integer) parameter (x : integer)
    return integer is
  begin
    return f(x);
  end function;
  function a1 is new ap generic map (f [real return real] => inc);
begin
  u1 : entity work.sub generic map (n [integer] => 1, g => inc);
  u2 : entity work.sub generic map (n => 1, g [bit return bit] => inc);
end architecture;
