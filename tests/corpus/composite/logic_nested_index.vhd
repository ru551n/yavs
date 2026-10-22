-- difftest: top=logic_nested_index std=2008
-- Elements of rows of arrays of logic vectors (`a(i)(j)`, `a(i)(h downto l)(j)`), in
-- variables, parameters and constants, descending and ascending rows and outer ranges, a
-- 2-D logic matrix and the element count of nested and multi-dimensional logic arrays.
library ieee;
use ieee.std_logic_1164.all;

entity logic_nested_index is
end entity;

architecture t of logic_nested_index is
  type mem_t is array (natural range <>) of std_logic_vector;
  type up_t is array (integer range <>) of std_logic_vector;
  type mat_t is array (natural range <>, natural range <>) of std_logic;

  function ones(m : mem_t) return natural is
    variable n : natural := 0;
  begin
    for i in m'range loop
      for j in m(i)'range loop
        if m(i)(j) = '1' then
          n := n + 1;
        end if;
      end loop;
    end loop;
    return n;
  end function;

  function transpose(m : mem_t) return mat_t is
    variable r : mat_t(m'range, m(m'left)'range);
  begin
    for i in m'range loop
      for j in m(i)'range loop
        r(i, j) := m(i)(j);
      end loop;
    end loop;
    return r;
  end function;

  function sum_slice(m : up_t; i : integer) return natural is
    variable n : natural := 0;
  begin
    for j in 2 to 4 loop
      if m(i)(2 to 5)(j) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  constant c : mem_t(0 to 2)(3 downto 0) := ("1010", "0111", "1000");
begin
  process
    variable v  : mem_t(3 downto 0)(7 downto 0);
    variable u  : up_t(-2 to 1)(0 to 7);
    variable m  : mat_t(3 downto 0, 7 downto 0);
    variable e  : mem_t(0 downto 1)(3 downto 0);
    variable cnt : natural;
  begin
    for i in v'range loop
      for j in v(i)'range loop
        if (i + j) mod 3 = 0 then
          v(i)(j) := '1';
        else
          v(i)(j) := '0';
        end if;
      end loop;
    end loop;
    assert v(3) = "01001001" report "v(3) " & to_string(v(3)) severity failure;
    assert v(0) = "01001001" report "v(0) " & to_string(v(0)) severity failure;
    assert ones(v) = 11 report "ones(v) " & integer'image(ones(v)) severity failure;
    m := transpose(v);
    assert m(2, 7) = v(2)(7) and m(3, 0) = v(3)(0) and m(1, 5) = v(1)(5) severity failure;
    assert ones(c) = 6 report "ones(c) " & integer'image(ones(c)) severity failure;
    assert c(1)(2) = '1' and c(1)(3) = '0' and c(2)(3) = '1' report "c" severity failure;
    for i in u'range loop
      u(i) := (others => '0');
      u(i)(i + 2) := '1';
    end loop;
    assert u(-1) = "01000000" report "u(-1) " & to_string(u(-1)) severity failure;
    assert u(1)(3) = '1' and u(1)(2) = '0' severity failure;
    -- the row, then an element of a slice of the row
    cnt := 0;
    for i in u'range loop
      cnt := cnt + sum_slice(u, i);
    end loop;
    assert cnt = 2 report "sum_slice " & integer'image(cnt) severity failure;
    v(2)(7 downto 4)(5) := 'Z';
    assert v(2)(5) = 'Z' and v(2)(7 downto 4)(5) = 'Z' report "slice element" severity failure;
    assert v(2)(7 downto 4)(4) = v(2)(4) severity failure;
    assert e'length = 0 report "e" severity failure;
    report "done";
    wait;
  end process;
end architecture;
