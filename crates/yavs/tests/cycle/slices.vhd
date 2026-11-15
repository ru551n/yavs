-- Slices whose bounds come from variables (`W` is replaced by the width): bounds a loop parameter
-- gives through two variables (a register file's field write), two variables chosen at run time
-- with a fixed distance (kept out: the distance is not followed), one variable on both bounds,
-- variables known on every path (both branches, a `case`, a procedure's `inout` formal, a
-- function's locals) and not known (branches that differ, a loop that may exit, a register's
-- state variable with an initial value).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_slices is
end entity;

architecture a of cycle_slices is
  function to_slv (v : std_ulogic_vector) return std_ulogic_vector is
    variable lo, hi : integer;
    variable r : std_ulogic_vector(v'length - 1 downto 0);
  begin
    lo := 0;
    hi := lo + v'length - 1;
    r(hi downto lo) := v;
    return r;
  end function;

  procedure bump (variable k : inout integer) is
  begin
    k := k + 1;
  end procedure;

  signal clk, c : std_ulogic := '0';
  signal x, mt : std_ulogic_vector(4 * W - 1 downto 0) := (others => '0');
  signal sel : unsigned(1 downto 0) := "00";
  signal mask : std_ulogic_vector(3 downto 0) := "0000";
  signal y1, y2, y3, y4, y5, y6 : std_ulogic_vector(W - 1 downto 0);
begin
  merge : process (all)
    variable hi, lo : integer := 0;
    variable t : std_ulogic_vector(4 * W - 1 downto 0);
  begin
    t := x;
    for i in 0 to 3 loop
      if mask(i) = '1' then
        lo := i * W;
        hi := lo + W - 1;
        t(hi downto lo) := not x(hi downto lo);
      end if;
    end loop;
    mt <= t;
  end process;

  apart : process (all)
    variable hi, lo : integer;
  begin
    lo := to_integer(sel) * W;
    hi := lo + W - 1;
    y1 <= x(hi downto lo);
  end process;

  one : process (all)
    variable lo : integer;
  begin
    lo := to_integer(sel) * W;
    y2 <= x(lo + W - 1 downto lo);
  end process;

  known : process (all)
    variable k : integer;
  begin
    if c = '1' then
      k := 1;
    else
      k := 1;
    end if;
    case sel is
      when "00" => k := k + 1;
      when others => k := 2;
    end case;
    bump(k);
    y3 <= to_slv(x(k * W - 1 downto (k - 1) * W));
  end process;

  unknown : process (all)
    variable k : integer;
  begin
    if c = '1' then
      k := 0;
    else
      k := 1;
    end if;
    y4 <= x(k * W + W - 1 downto k * W);
    for j in 0 to 3 loop
      k := j;
      exit when to_integer(sel) = j;
    end loop;
    y5 <= x(k * W + W - 1 downto k * W);
  end process;

  reg : process (clk)
    variable k : integer range 0 to 3 := 0;
  begin
    if rising_edge(clk) then
      y6 <= x(k * W + W - 1 downto k * W);
      k := (k + 1) mod 4;
    end if;
  end process;

  main : process
    variable s : unsigned(4 * W - 1 downto 0) := (others => '0');
  begin
    for cyc in 0 to 15 loop
      s := rotate_left(s, 5) xor resize(to_unsigned(cyc * 97 + 13, 16), 4 * W);
      s(s'left) := not s(0);
      x <= std_ulogic_vector(s);
      sel <= to_unsigned(cyc mod 4, 2);
      c <= '1' when cyc mod 3 = 0 else '0';
      mask <= std_ulogic_vector(to_unsigned((cyc * 5) mod 16, 4));
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      report "cyc=" & integer'image(cyc) & " mt=" & to_hstring(mt) & " y1=" & to_hstring(y1)
        & " y2=" & to_hstring(y2) & " y3=" & to_hstring(y3) & " y4=" & to_hstring(y4)
        & " y5=" & to_hstring(y5) & " y6=" & to_hstring(y6);
    end loop;
    wait;
  end process;
end architecture;
