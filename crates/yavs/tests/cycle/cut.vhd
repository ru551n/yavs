-- Members large enough to be cut into several compiled functions: a variable of a combinational
-- process live across every part of two unrolled loops, the `if` around them cut inside its
-- branches, branches inside the loops, and a range check after the loop of a clocked process (in
-- its last part). Conditions written inside their own branches: a loop left by `exit when` (later
-- iterations behind its flag) and an `if` on a variable it clears. LIMIT below 20000 makes the
-- check fail.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_cut is
  generic (N : natural := 1200; LIMIT : natural := 1000000);
end entity;

architecture a of cycle_cut is
  signal clk : std_logic := '0';
  signal x : unsigned(31 downto 0) := x"12345678";
  signal sel : std_logic := '0';
  signal y, z : unsigned(31 downto 0) := (others => '0');
  signal cnt : natural := 0;
  signal k : natural range 0 to LIMIT := 0;
begin
  p_x : process (clk)
  begin
    if rising_edge(clk) then
      x <= x(30 downto 0) & (x(31) xor x(21) xor x(1) xor x(0));
      sel <= x(3);
      cnt <= cnt + 1;
    end if;
  end process;

  p_comb : process (all)
    variable acc : unsigned(31 downto 0);
  begin
    acc := x;
    if sel = '1' then
      for i in 0 to N - 1 loop
        if acc(i mod 32) = '1' then
          acc := acc + to_unsigned(i, 32);
        else
          acc := (acc(28 downto 0) & acc(31 downto 29)) xor to_unsigned(i * 7919 mod 65536, 32);
        end if;
      end loop;
    else
      for i in 0 to N - 1 loop
        acc := acc + shift_right(acc, 3) + to_unsigned(i, 32);
        exit when acc(31 downto 26) = "101101";
      end loop;
    end if;
    y <= acc;
  end process;

  p_clk : process (clk)
    variable v : natural;
    variable go : boolean;
  begin
    if rising_edge(clk) then
      v := 0;
      go := sel = '1';
      if go then
        go := false;
        for i in 0 to N - 1 loop
          if y(i mod 32) = '1' then
            v := v + 1;
          end if;
          if go then
            v := v + 7;
          end if;
        end loop;
      else
        v := 3;
      end if;
      z <= y xor to_unsigned(v, 32);
      k <= cnt * 1000;
    end if;
  end process;

  main : process
  begin
    for c in 1 to 12 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "c=" & integer'image(c) & " y=" & to_hstring(y) & " z=" & to_hstring(z) & " k=" & integer'image(k);
    end loop;
    wait;
  end process;
end architecture;
