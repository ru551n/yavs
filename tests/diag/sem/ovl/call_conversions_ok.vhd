-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.1
-- Legal conversions in calls: a formal conversion of an out parameter (function or type
-- conversion), both conversions of an inout one, and an actual conversion of an in
-- variable.
entity e is end entity;
architecture a of e is
  type rgb is (red, green, blue);
  function to_r(i : integer) return real is begin return real(i); end function;
  function to_i(r : real) return integer is begin return integer(r); end function;
  function to_c(i : integer) return rgb is begin return rgb'val(i mod 3); end function;
  function to_r(c : rgb) return real is begin return real(rgb'pos(c)); end function;
  procedure seti(variable x : out integer) is begin x := 3; end procedure;
  procedure setc(variable c : out rgb) is begin c := blue; end procedure;
  procedure inc(variable x : inout integer) is begin x := x + 1; end procedure;
  procedure geti(variable x : in integer) is begin end procedure;
begin
  process
    variable r : real := 1.0;
    variable c : rgb;
  begin
    seti(to_r(x) => r);
    seti(real(x) => r);
    setc(to_r(c) => r);
    inc(to_r(x) => to_i(r));
    inc(real(x) => integer(r));
    geti(to_i(r));
    wait;
  end process;
end architecture;
