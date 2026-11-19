-- The index ranges of logical operator results given to a function that walks its argument's
-- range (`for i in d'range`, `d'left`), against the normal mode (`W` replaced by a width): the
-- `std_logic_1164` operators (`1 to n`), `numeric_std`'s (`n - 1 downto 0`) and the predefined
-- ones of BIT vectors (the left operand's range).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_oprange_pkg is
  function weigh(d : std_ulogic_vector) return natural;
  function weigh(d : bit_vector) return natural;
  function weigh(d : unsigned) return natural;
end package;
package body cycle_oprange_pkg is
  -- Each one set counts its distance from `d'left`, so the range shows.
  function weigh(d : std_ulogic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in d'range loop
      if d(i) = '1' then
        n := n + abs (i - d'left) + 1;
      end if;
    end loop;
    return n + d'left;
  end function;
  function weigh(d : bit_vector) return natural is
    variable n : natural := 0;
  begin
    for i in d'range loop
      if d(i) = '1' then
        n := n + abs (i - d'left) + 1;
      end if;
    end loop;
    return n + d'left;
  end function;
  function weigh(d : unsigned) return natural is
    variable n : natural := 0;
  begin
    for i in d'range loop
      if d(i) = '1' then
        n := n + abs (i - d'left) + 1;
      end if;
    end loop;
    return n + d'left;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_oprange_pkg.all;
entity cycle_oprange is
end entity;
architecture t of cycle_oprange is
  signal clk : std_ulogic := '0';
  signal a, b : std_ulogic_vector(W + 2 downto 3) := (others => '0');
  signal ba, bb : bit_vector(4 to W + 3) := (others => '0');
  signal ua, ub : unsigned(W + 1 downto 2) := (others => '0');
  signal n1, n2, n3, n4, n5, n6 : natural;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      n1 <= weigh(a and b);
      n2 <= weigh(not a);
      n3 <= weigh(ba xor bb);
      n4 <= weigh(not ba);
      n5 <= weigh(ua or ub);
      n6 <= weigh(not ub);
    end if;
  end process;
  main : process
    variable s : std_ulogic_vector(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 10 loop
      s := s(14 downto 0) & (s(15) xor s(13) xor s(12) xor s(10));
      a <= a(W + 1 downto 3) & s(0);
      b <= s(1) & b(W + 2 downto 4);
      ba <= ba(5 to W + 3) & to_bit(s(2));
      bb <= to_bit(s(3)) & bb(4 to W + 2);
      ua <= ua(W downto 2) & s(4);
      ub <= s(5) & ub(W + 1 downto 3);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " " & integer'image(n1) & " " & integer'image(n2) & " "
        & integer'image(n3) & " " & integer'image(n4) & " " & integer'image(n5) & " " & integer'image(n6);
    end loop;
    wait;
  end process;
end architecture;
