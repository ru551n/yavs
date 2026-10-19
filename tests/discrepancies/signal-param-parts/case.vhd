-- (from the differential corpus)
-- ref=hand: signal formals of subprograms associated element by element, each element in
-- another signal (LRM 6.5.7.1, #71): reads, drives, 'event, 'active, 'last_event, wait on,
-- and passing the formal on. The first reference simulator does not support this form of
-- parameter; .expected is hand-written, the second reference simulator gives the same values
-- and the asserts check them.
library ieee;
use ieee.std_logic_1164.all;
entity signal_param_parts is end;
architecture t of signal_param_parts is
  type iarr is array (natural range <>) of integer;
  signal s0, s1 : bit := '0';
  signal l3, l2, l1, l0 : std_logic := 'U';
  signal i0, i1, i2 : integer := 1;

  procedure drive2(signal x : out bit_vector(1 downto 0); v : bit_vector(1 downto 0)) is
  begin
    x <= v;
  end procedure;

  procedure show(signal x : in std_logic_vector) is
  begin
    report "show " & integer'image(x'left) & " " & integer'image(x'right) & " "
      & to_string(x) & " event=" & boolean'image(x'event);
  end procedure;

  procedure set_bit(signal x : out std_logic_vector(3 downto 0); i : natural; v : std_logic) is
  begin
    x(i) <= v;
  end procedure;

  procedure pass_on(signal x : inout std_logic_vector(3 downto 0); v : std_logic_vector) is
  begin
    set_bit(x, 3, v(3));
    for k in 2 downto 0 loop
      x(k) <= v(k) after 1 ns;
    end loop;
    wait for 0 ns;
    show(x);
    wait for 1 ns;
    show(x);
  end procedure;

  procedure sum(signal x : in iarr(0 to 2)) is
    variable t : integer := 0;
  begin
    for k in x'range loop
      t := t + x(k);
    end loop;
    report "sum " & integer'image(t) & " last_event " & time'image(x'last_event);
    assert t = 7 and x'last_event = 1 ns;
  end procedure;

  procedure watch(signal x : in iarr(0 to 2)) is
  begin
    wait on x;
    report "watch " & integer'image(x(0)) & " " & integer'image(x(1)) & " "
      & integer'image(x(2)) & " active=" & boolean'image(x'active);
    assert x = (1, 5, 30) and x'active;
  end procedure;
begin
  p1 : process
  begin
    drive2(x(1) => s1, x(0) => s0, v => "10");
    wait for 0 ns;
    report "s1=" & bit'image(s1) & " s0=" & bit'image(s0);
    assert s1 = '1' and s0 = '0';
    drive2(x(0) => s1, x(1) => s0, v => "10");
    wait for 0 ns;
    report "s1=" & bit'image(s1) & " s0=" & bit'image(s0);
    assert s1 = '0' and s0 = '1';
    pass_on(x(3) => l3, x(2) => l2, x(1) => l1, x(0) => l0, v => "1010");
    report "l=" & std_logic'image(l3) & std_logic'image(l2) & std_logic'image(l1)
      & std_logic'image(l0);
    assert l3 = '0' and l2 = '1' and l1 = '0' and l0 = '1';
    wait for 2 ns;
    i1 <= 5;
    wait for 1 ns;
    sum(x(0) => i0, x(1) => i1, x(2) => i2);
    wait;
  end process;

  p2 : process
  begin
    wait for 5 ns;
    watch(x(2) => i0, x(1) => i1, x(0) => i2);
    wait;
  end process;

  p3 : process
  begin
    wait for 6 ns;
    i0 <= 30;
    wait;
  end process;
end architecture;
