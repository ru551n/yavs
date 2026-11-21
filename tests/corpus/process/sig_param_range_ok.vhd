-- difftest: top=sig_param_range_ok std=2008
-- Signal formals whose actual is narrower than the formal, driven only with values in range,
-- run as before in every form the checked call takes (#1181): out, inout, a part, a formal
-- passed on, a concurrent call, a procedure with a nested function, a procedure declared in
-- the process.
entity sig_param_range_ok is end entity;
architecture a of sig_param_range_ok is
  subtype digit is integer range 0 to 9;
  type digits is array (0 to 3) of digit;
  signal s, c : digit := 0;
  signal v : digits := (others => 0);
  signal go : integer := 0;
  procedure drv (signal o : out integer; x : integer) is begin o <= x; end procedure;
  procedure inc (signal o : inout integer) is begin o <= o + 1; end procedure;
  procedure fwd (signal o : inout integer) is begin inc(o); end procedure;
  procedure helper (signal o : out integer; x : integer) is
    function twice (y : integer) return integer is begin return 2 * y; end function;
  begin
    o <= twice(x);
  end procedure;
begin
  drv(c, go);
  process
    procedure local (signal o : out integer) is begin o <= 3; end procedure;
  begin
    drv(s, 5);
    wait for 1 ns;
    inc(s);
    wait for 1 ns;
    fwd(s);
    wait for 1 ns;
    report "s=" & integer'image(s);
    drv(v(0), 1);
    drv(v(1), 3);
    drv(v(2), 5);
    drv(v(3), 7);
    wait for 1 ns;
    report "v=" & integer'image(v(0)) & integer'image(v(1)) & integer'image(v(2)) & integer'image(v(3));
    helper(s, 4);
    wait for 1 ns;
    report "s=" & integer'image(s);
    local(s);
    go <= 9;
    wait for 1 ns;
    report "s=" & integer'image(s) & " c=" & integer'image(c);
    wait;
  end process;
end architecture;
