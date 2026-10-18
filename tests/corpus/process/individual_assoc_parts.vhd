-- difftest: top=individual_assoc_parts std=2008 ref=hand
-- ref=hand: individual association of parameters by slices and record elements (LRM
-- 6.5.7.1). The reference simulators crash on slice formal parts and one copies logic
-- elements back in reverse; .expected is hand-written, the asserts check the values.
library ieee;
use ieee.std_logic_1164.all;
entity individual_assoc_parts is end;
architecture t of individual_assoc_parts is
  type rec is record
    a : integer;
    b : bit;
  end record;
  signal tl : bit := '0';
  signal hi : bit_vector(7 downto 0) := x"a5";
  signal wide : bit_vector(0 to 3) := "0000";

  procedure show(signal x : in bit_vector) is
  begin
    report "show " & integer'image(x'left) & " " & integer'image(x'right) & " "
      & to_string(x);
  end procedure;

  procedure drive_all(signal x : out bit_vector; v : bit) is
  begin
    for i in x'range loop
      x(i) <= v;
    end loop;
  end procedure;

  procedure fill(variable v : out bit_vector(3 downto 0); x : bit_vector(3 downto 0)) is
  begin
    v := x;
  end procedure;

  procedure flip(variable v : inout std_ulogic_vector) is
  begin
    report "flip " & to_string(v);
    v := not v;
  end procedure;

  procedure bump(variable r : inout rec) is
  begin
    r.a := r.a + 1;
    r.b := not r.b;
  end procedure;
begin
  process
    variable p, q : bit_vector(1 downto 0);
    variable n : integer := 7;
    variable f : bit := '0';
    variable l0 : std_ulogic := '1';
    variable l1 : std_ulogic := 'Z';
  begin
    show(x(2 to 5) => hi(5 downto 2));
    drive_all(x(2 to 3) => wide(1 to 2), v => '1');
    fill(v(3 downto 2) => p, v(1 downto 0) => q, x => "1001");
    assert p = "10" and q = "01" report "fill failed" severity failure;
    flip(v(1) => l1, v(0) => l0);
    assert l0 = '0' and l1 = 'X' report "flip failed" severity failure;
    bump(r.b => f, r.a => n);
    assert n = 8 and f = '1' report "bump failed" severity failure;
    wait for 1 ns;
    assert wide = "0110" report "drive_all failed" severity failure;
    report "done";
    wait;
  end process;
end architecture;
