-- (from the differential corpus)
-- ref=hand: signal formals of subprograms associated by slices and elements in different
-- signals (LRM 6.5.7.1, #71), with slices of the formal across its parts and an
-- unconstrained formal taking its bounds from the parts. Both reference simulators crash on
-- slice formal parts of signal parameters; .expected is hand-written from the LRM, the
-- asserts check the values.
library ieee;
use ieee.std_logic_1164.all;
entity signal_param_slices is end;
architecture t of signal_param_slices is
  type iarr is array (natural range <>) of integer;
  signal lv : std_logic_vector(2 downto 0) := "UUU";
  signal l0 : std_logic := 'U';
  signal i0 : integer := 1;
  signal iv : iarr(0 to 1) := (2, 3);

  procedure show(signal x : in std_logic_vector) is
  begin
    report "show " & integer'image(x'left) & " " & integer'image(x'right) & " "
      & to_string(x) & " event=" & boolean'image(x'event);
  end procedure;

  procedure pass_on(signal x : inout std_logic_vector(3 downto 0); v : std_logic_vector) is
  begin
    x(3) <= v(0);
    x(2 downto 0) <= v(1 to 3) after 1 ns;
    wait for 0 ns;
    show(x);
    wait for 1 ns;
    show(x);
    show(x(1 downto 0));
    assert x = "0101" and x(2 downto 1) = "10";
  end procedure;

  procedure sum(signal x : in iarr) is
    variable t : integer := 0;
  begin
    for k in x'range loop
      t := t + x(k);
    end loop;
    report "sum " & integer'image(x'left) & " to " & integer'image(x'right) & " = "
      & integer'image(t);
    assert t = 6;
  end procedure;

  procedure set_all(signal x : out iarr(0 to 2); v : integer) is
  begin
    x <= (v, v + 1, v + 2);
  end procedure;
begin
  process
  begin
    pass_on(x(3 downto 1) => lv, x(0) => l0, v => "0101");
    report "lv=" & to_string(lv) & " l0=" & std_logic'image(l0);
    assert lv = "010" and l0 = '1';
    sum(x(5) => i0, x(6 to 7) => iv);
    set_all(x(1 to 2) => iv, x(0) => i0, v => 10);
    wait for 0 ns;
    report "i0=" & integer'image(i0) & " iv=" & integer'image(iv(0)) & ","
      & integer'image(iv(1));
    assert i0 = 10 and iv = (11, 12);
    wait;
  end process;
end architecture;
