-- difftest: top=loop_invariant
-- Nested loops over arrays whose index and descriptor arithmetic does not change in the inner
-- loop (#526): matrices from arrays of vectors in both directions, null inner ranges, a check
-- inside a branch no pass takes, and an inner loop that writes a variable it also indexes with.
library ieee;
use ieee.std_logic_1164.all;

entity loop_invariant is end;

architecture a of loop_invariant is
  type mem_t is array (natural range <>) of std_logic_vector;
  type mat_t is array (natural range <>, natural range <>) of std_logic;
  type ints is array (integer range <>) of integer;

  function to_mat(m : mem_t) return mat_t is
    variable r : mat_t(m'range, m(m'left)'range);
  begin
    for i in m'range loop
      for j in m(i)'range loop
        r(i, j) := m(i)(j);
      end loop;
    end loop;
    return r;
  end function;

  function sum(m : mat_t) return natural is
    variable n : natural := 0;
  begin
    for i in m'range(1) loop
      for j in m'range(2) loop
        if m(i, j) = '1' then
          n := n + i * 7 + j;
        end if;
      end loop;
    end loop;
    return n;
  end function;

  -- `v(k)` is out of range, but the branch that reads it is never taken.
  function skipped(v : ints; k : integer; n : natural) return integer is
    variable s : integer := 0;
  begin
    for i in 1 to n loop
      for j in v'range loop
        if j = v'high + 1 then
          s := s + v(k);
        end if;
        s := s + v(j) * i;
      end loop;
    end loop;
    return s;
  end function;

  -- The inner loop runs zero times: its invariant `v(k)` is never evaluated.
  function empty(v : ints; k : integer) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      for j in 1 to i - 100 loop
        s := s + v(k) + j;
      end loop;
      s := s + v(i);
    end loop;
    return s;
  end function;

  -- The inner loop writes the element it reads through an index that stays the same.
  function prefix_sums(v : ints) return ints is
    variable r : ints(v'range) := v;
  begin
    for i in r'range loop
      for j in r'low to i - 1 loop
        r(i) := r(i) + v(j);
      end loop;
    end loop;
    return r;
  end function;
begin
  process
    variable up : mem_t(0 to 5)(7 downto 0);
    variable dn : mem_t(9 downto 6)(2 to 4);
    variable e  : mem_t(1 to 3)(1 to 0);
    variable v  : ints(-2 to 3) := (5, -1, 4, 0, 9, 2);
    variable p  : ints(-2 to 3);
  begin
    for i in up'range loop
      up(i) := (others => '0');
      up(i)(i) := '1';
      up(i)(7 - i) := '1';
    end loop;
    for i in dn'range loop
      dn(i) := (others => '1');
      dn(i)(2 + i mod 3) := '0';
    end loop;
    report "up " & integer'image(sum(to_mat(up)));
    report "dn " & integer'image(sum(to_mat(dn)));
    report "e " & integer'image(to_mat(e)'length(1)) & " " & integer'image(to_mat(e)'length(2));
    assert to_mat(up)(3, 4) = '1' and to_mat(up)(3, 5) = '0' severity failure;
    assert to_mat(dn)(8, 3) = '1' and to_mat(dn)(8, 4) = '0' severity failure;
    report "skipped " & integer'image(skipped(v, 100, 3));
    report "empty " & integer'image(empty(v, 100));
    p := prefix_sums(v);
    for i in p'range loop
      report "p(" & integer'image(i) & ") " & integer'image(p(i));
    end loop;
    wait;
  end process;
end;
