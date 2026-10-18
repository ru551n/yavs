-- difftest: top=tb_out_param_elements std=2008
-- One process calls a procedure with a `signal out` parameter on several constant parts of
-- one signal: each actual's longest static prefix gets its own driver (LRM 14.7.2, #231).
library ieee;
use ieee.std_logic_1164.all;
entity tb_out_param_elements is end entity;
architecture a of tb_out_param_elements is
  type slv_vector is array (natural range <>) of std_logic_vector(1 to 5);
  signal s12, s24, s25, s13, s14, s15, s35, s51 : slv_vector(1 to 5) := (others => "00000");
  signal v : std_logic_vector(1 to 10) := (others => '0');
  signal e : std_logic_vector(0 to 7) := (others => '0');
  signal w : slv_vector(1 to 6) := (others => "00000");

  procedure apply(constant s : in string; signal data : out std_logic_vector) is
    variable j : integer := data'left;
  begin
    while j <= data'high loop
      data(j) <= std_logic'value("'" & s(j - data'left + 1) & "'");
      j := j + 1;
    end loop;
  end procedure;

  procedure set(constant x : in std_logic; signal b : out std_logic) is
  begin
    b <= x;
  end procedure;

  procedure fill(constant x : in std_logic_vector; signal d : out slv_vector) is
  begin
    for i in d'range loop
      d(i) <= x;
    end loop;
  end procedure;

  function image(x : slv_vector) return string is
    variable r : string(1 to 6 * x'length) := (others => ' ');
  begin
    for i in x'range loop
      r(6 * (i - x'low) + 1 to 6 * (i - x'low) + 5) := to_string(x(i));
    end loop;
    return r;
  end function;
begin
  p12 : process begin apply("10101", s12(1)); apply("01011", s12(2)); wait; end process;
  p24 : process begin apply("10101", s24(2)); apply("01011", s24(4)); wait; end process;
  p25 : process begin apply("10101", s25(2)); apply("01011", s25(5)); wait; end process;
  p13 : process begin apply("10101", s13(1)); apply("01011", s13(3)); wait; end process;
  p14 : process begin apply("10101", s14(1)); apply("01011", s14(4)); wait; end process;
  p15 : process begin apply("10101", s15(1)); apply("01011", s15(5)); wait; end process;
  p35 : process begin apply("10101", s35(3)); apply("01011", s35(5)); wait; end process;
  p51 : process begin apply("10101", s51(5)); apply("01011", s51(1)); wait; end process;

  -- Slices of a vector, out of order, and single elements as scalar actuals.
  pv : process begin
    apply("110", v(6 to 8));
    apply("101", v(1 to 3));
    apply("11", v(9 to 10));
    wait;
  end process;
  pe : process begin
    set('1', e(6));
    set('1', e(1));
    set('1', e(3));
    wait;
  end process;
  -- Slices of an array-of-vector signal.
  pw : process begin
    fill("11100", w(5 to 6));
    fill("00111", w(1 to 2));
    wait;
  end process;

  check : process begin
    wait for 1 ns;
    report "s12 " & image(s12);
    report "s24 " & image(s24);
    report "s25 " & image(s25);
    report "s13 " & image(s13);
    report "s14 " & image(s14);
    report "s15 " & image(s15);
    report "s35 " & image(s35);
    report "s51 " & image(s51);
    report "v " & to_string(v);
    report "e " & to_string(e);
    report "w " & image(w);
    assert s13 = ("10101", "00000", "01011", "00000", "00000") severity failure;
    assert s51 = ("01011", "00000", "00000", "00000", "10101") severity failure;
    assert v = "1010011011" severity failure;
    assert e = "01010010" severity failure;
    assert w = ("00111", "00111", "00000", "00000", "11100", "11100") severity failure;
    wait;
  end process;
end architecture;
