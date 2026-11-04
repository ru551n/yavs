-- difftest: top=stack_cells_nested std=2008 ignore=order
-- ignore=order: several instances report in one delta cycle, in an order the LRM leaves open.
-- Procedures nested in a process of an instance and in each other, called from what may be a copy
-- of that process: open array parameters of both directions and null, a by-reference scalar and a
-- variable of the enclosing frame, more arguments than registers, bounds of the enclosing
-- subprogram's parameter read from inside, array results used as actuals.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;

entity stack_cells_inst is
  generic (K : natural; W : positive);
  port (clk : in std_logic; o : out natural);
end entity;

architecture a of stack_cells_inst is
  type col is (red, green, blue, black);
  type col_arr is array (col range <>) of natural;
  type int_arr is array (integer range <>) of integer;
  signal s : std_logic_vector(W - 1 downto 0) := (others => '0');
begin
  main : process (clk)
    variable total : natural := 0;
    variable mine : natural := K;
    variable up : std_logic_vector(0 to W - 1) := (others => '0');
    variable dn : std_logic_vector(W + 2 downto 3) := (others => '1');
    variable ints : int_arr(-2 to 3) := (5, -6, 7, -8, 9, -10);
    variable cols : col_arr(green to black) := (1, 2, 3);
    variable s1, s2 : positive := K + 1;
    variable rnd : real := 0.0;
    variable cnt : natural := 0;

    function bits(a : std_logic_vector) return natural is
      variable c : natural := 0;
    begin
      for i in a'range loop
        if a(i) = '1' then c := c + 1; end if;
      end loop;
      return c;
    end function;

    -- An array result with bounds from its parameter and from the instance.
    function rev(a : std_logic_vector) return std_logic_vector is
      variable r : std_logic_vector(a'length + W - 1 downto W);
      variable j : natural := r'right;
    begin
      for i in a'range loop
        r(j) := a(i);
        j := j + 1;
      end loop;
      return r;
    end function;

    function middle(a : std_logic_vector) return std_logic_vector is
    begin
      if a'length < 3 then
        if a'ascending then
          return a(a'left to a'left - 1);
        else
          return a(a'left downto a'left + 1);
        end if;
      elsif a'ascending then
        return a(a'left + 1 to a'right - 1);
      else
        return a(a'left - 1 downto a'right + 1);
      end if;
    end function;

    procedure note(variable x : inout natural; n : natural) is
    begin
      x := (x + n) mod 100003;
      total := (total + 1) mod 100003;
    end procedure;

    -- More cells than argument registers; bounds of its parameter read from a nested call.
    procedure work_on(a : in std_logic_vector; variable b : inout std_logic_vector;
                      lo, hi : in integer; variable t : inout natural;
                      c : in std_logic_vector; i : in int_arr) is
      variable here : natural := a'length;
      procedure inner(idx : integer; variable acc : inout natural) is
      begin
        acc := acc + a'length + b'length + c'length + here;
        if a'ascending then acc := acc + 1; end if;
        if b'ascending then acc := acc + 2; end if;
        if c'ascending then acc := acc + 4; end if;
        if idx >= a'low and idx <= a'high then
          if a(idx) = '1' then acc := acc + 8; end if;
        end if;
        if idx >= b'low and idx <= b'high then
          b(idx) := not b(idx);
        end if;
        if idx >= i'low and idx <= i'high then
          acc := acc + abs i(idx);
        end if;
        here := here + 1;
        total := (total + a'left + b'right) mod 100003;
      end procedure;
    begin
      for j in lo to hi loop
        inner(j, t);
      end loop;
      t := (t + here + i'length + i'left + i'right) mod 100003;
    end procedure;

    procedure cols_sum(a : in col_arr; variable t : inout natural) is
    begin
      for c in a'range loop
        t := (t + a(c) * (col'pos(c) + 1)) mod 100003;
      end loop;
      t := t + col'pos(a'left) * 10 + col'pos(a'right) * 100;
    end procedure;
  begin
    if rising_edge(clk) then
      cnt := cnt + 1;
      work_on(up, dn, 0, W + 3, mine, s, ints);
      work_on(dn, up, cnt mod W, cnt mod W + 2, mine, up(1 to 0), ints(2 to 1));
      work_on(rev(up), dn, 3, 4, mine, middle(dn), ints(0 to 1));
      work_on(middle(middle(up)), up, 1, 1, mine, rev(middle(dn)), ints);
      note(mine, bits(rev(rev(dn))));
      note(mine, bits(middle(up(0 to 0))));
      note(mine, bits(middle(s)));
      cols_sum(cols, mine);
      cols_sum(cols(blue to blue), mine);
      cols_sum(cols(blue to green), mine);
      uniform(s1, s2, rnd);
      if rnd > 0.5 then note(mine, 3); end if;
      s(cnt mod W) <= not s(cnt mod W);
      o <= mine + total;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity stack_cells_nested is
end entity;

architecture a of stack_cells_nested is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal o0, o1, o2 : natural := 0;
begin
  clk <= not clk after 5 ns when n < 300 else '0';
  count : process (clk)
  begin
    if rising_edge(clk) then n <= n + 1; end if;
  end process;
  i0 : entity work.stack_cells_inst generic map (0, 5) port map (clk, o0);
  i1 : entity work.stack_cells_inst generic map (1, 9) port map (clk, o1);
  i2 : entity work.stack_cells_inst generic map (2, 70) port map (clk, o2);
  done : process (n)
  begin
    if n mod 50 = 49 then
      report integer'image(o0) & " " & integer'image(o1) & " " & integer'image(o2);
    end if;
  end process;
end architecture;
