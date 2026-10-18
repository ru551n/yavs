-- difftest: top=signal_bounds_init std=2019
-- VHDL-2019 signals of unconstrained array subtypes that take their bounds from the
-- initial value (LRM 6.4.2.3), in an architecture, a block, a generate and a package.
package sbi_pkg is
  signal pk : bit_vector := "1100";
  function ones(n : natural) return bit_vector;
end package;

package body sbi_pkg is
  function ones(n : natural) return bit_vector is
    variable r : bit_vector(n - 1 downto 0) := (others => '1');
  begin
    return r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.sbi_pkg.all;

entity signal_bounds_init is
  generic (W : natural := 3);
end entity;

architecture a of signal_bounds_init is
  constant c : std_logic_vector(7 downto 4) := "10ZX";
  type slv_array is array (natural range <>) of std_logic_vector;
  signal s : std_logic_vector := c;
  signal t : bit_vector := (3 to 5 => '1');
  signal f : bit_vector := ones(W + 2);
  signal arr : slv_array := ("01", "10", "11");
  signal str : string := "hello";
begin
  main : process
  begin
    assert s'left = 7 and s'right = 4 and s'length = 4 and not s'ascending;
    assert s = "10ZX";
    assert t'left = 3 and t'right = 5 and t = "111";
    assert f'left = 4 and f'right = 0 and f = "11111";
    assert arr'left = 0 and arr'length = 3 and arr(0)'length = 2 and arr(1)'left = 0;
    assert arr(2) = "11";
    assert pk'length = 4 and pk = "1100";
    assert str'length = 5;
    report "s = " & to_string(s) & " t = " & to_string(t) & " f = " & to_string(f)
      & " arr(1) = " & to_string(arr(1)) & " pk = " & to_string(pk) & " str = " & str;
    s <= "0101";
    t <= "010";
    f(2 downto 1) <= "00";
    pk <= "0011";
    str <= "world";
    wait for 1 ns;
    assert s = "0101" and t = "010" and f = "11001" and pk = "0011" and str = "world";
    report "s = " & to_string(s) & " t = " & to_string(t) & " f = " & to_string(f)
      & " pk = " & to_string(pk) & " str = " & str;
    wait;
  end process;

  blk : block
    signal b : std_logic_vector := "0110ZX";
  begin
    process
    begin
      assert b'length = 6 and b = "0110ZX";
      b <= (b'range => '0');
      wait for 1500 ps;
      assert b = "000000";
      report "blk: b'length = " & integer'image(b'length) & " b = " & to_string(b);
      wait;
    end process;
  end block;

  gen : for i in 1 to 2 generate
    signal g : bit_vector := ones(i);
  begin
    process
    begin
      wait for i * 2 ns;
      assert g'length = i;
      report "gen " & integer'image(i) & ": g = " & to_string(g);
      wait;
    end process;
  end generate;
end architecture;
