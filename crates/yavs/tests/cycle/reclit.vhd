-- Literal aggregates of arrays of records in cycle mode against the normal mode (`W` is replaced by
-- the width of a field): a register reset to a table whose elements differ (a logic field, a vector
-- field, a boolean and a nested record), one element written at a run-time index between resets.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_reclit is
end entity;
architecture a of cycle_reclit is
  type inner_t is record
    en : boolean;
    k : std_ulogic_vector(2 downto 0);
  end record;
  type entry_t is record
    valid : std_ulogic;
    data : std_ulogic_vector(W - 1 downto 0);
    sub : inner_t;
  end record;
  type table_t is array (0 to 3) of entry_t;
  constant init_c : table_t := (
    0 => (valid => '1', data => std_ulogic_vector(to_unsigned(5, W)), sub => (en => true, k => "001")),
    1 => (valid => '0', data => (others => '1'), sub => (en => false, k => "110")),
    2 => (valid => '1', data => (0 => '1', others => '0'), sub => (en => true, k => "100")),
    3 => (valid => '0', data => (W - 1 => '1', others => '0'), sub => (en => false, k => "011"))
  );
  signal clk, rst : std_ulogic := '0';
  signal idx : unsigned(1 downto 0) := "00";
  signal din : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal t : table_t;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        t <= init_c;
      else
        t(to_integer(idx)).data <= din;
        t(to_integer(idx)).sub.en <= not t(to_integer(idx)).sub.en;
      end if;
    end if;
  end process;

  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
    variable line : string(1 to 4);
  begin
    for c in 1 to 12 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      rst <= '1' when c mod 5 = 1 else '0';
      idx <= r(1 downto 0);
      din <= std_ulogic_vector(resize(r, W));
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      for i in 0 to 3 loop
        line(i + 1) := '1' when t(i).sub.en else '0';
      end loop;
      report "c=" & integer'image(c) & " v=" & std_ulogic'image(t(0).valid) & std_ulogic'image(t(3).valid)
        & " d=" & to_hstring(t(0).data) & "," & to_hstring(t(1).data) & "," & to_hstring(t(2).data)
        & "," & to_hstring(t(3).data) & " en=" & line & " k=" & to_string(t(0).sub.k)
        & to_string(t(1).sub.k) & to_string(t(2).sub.k) & to_string(t(3).sub.k);
    end loop;
    wait;
  end process;
end architecture;
