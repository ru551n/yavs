-- yavs-test: std=2019
-- expect: E1311 9:47 formal `x` of procedure `pr` has no actual and has no default
-- expect: E1502 16:36 `q` has an anonymous type and needs an actual to bind it
-- LRM 2019 6.5.3: the type of such an object is the type of its actual
package p is
  procedure pr (x : in type is (<>));
end package;
package body p is
  procedure pr (x : in type is (<>)) is begin pr; end procedure;
end package body;
entity e is port (q : in type is (<>)); end entity;
architecture a of e is begin end architecture;
entity top is end;
architecture a of top is
begin
  u : entity work.e port map (q => open);
end architecture;
