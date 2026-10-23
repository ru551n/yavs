-- yavs-test: std=1993
-- expect: E1360 11:10 cannot read parameter `x` of mode `out`
-- help: declare it `inout`
-- LRM 1993 4.3.2
entity e is end entity;
architecture a of e is
  procedure p (x : out integer) is
    variable n : integer;
  begin
    x := 1;
    n := x;
  end procedure;
begin
end architecture;
