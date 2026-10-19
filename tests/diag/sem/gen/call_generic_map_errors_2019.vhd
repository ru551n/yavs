-- yavs-test: std=2019
-- expect: E1502 24:5 generic `im` has no actual and no default
-- expect: E1201 25:50 the actual of a generic type must be a subtype
-- expect: E1501 26:26 `plain` is not a generic subprogram
-- expect: E1100 27:45 no generic `u`
-- expect: E1502 27:26 generic type `t` has no actual
-- expect: E1301 28:26 the instance takes an integer
-- expect: E1302 29:26 two generic subprograms `twin`
-- LCS-2016-049: errors of a generic map in a call
entity e is end entity;
architecture a of e is
  procedure show generic (type t; function im (x : t) return string)
    parameter (x : t) is
  begin
    report im(x);
  end procedure;
  function ident generic (type t) parameter (x : t) return t is begin return x; end function;
  function plain (x : integer) return integer is begin return x; end function;
  function twin generic (type t) parameter (x : t) return t is begin return x; end function;
  function twin generic (type t) parameter (x, y : t) return t is begin return x; end function;
begin
  process
  begin
    show generic map (t => integer) (6);
    report integer'image(ident generic map (t => 5) (4));
    report integer'image(plain generic map (t => integer) (4));
    report integer'image(ident generic map (u => integer) (4));
    report integer'image(ident generic map (t => integer) (true));
    report integer'image(twin generic map (t => integer) (1));
    wait;
  end process;
end architecture;
