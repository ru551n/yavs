-- difftest: top=redrive_after
-- A zero-delay assignment followed in the same activation by one with `after`, and the reverse,
-- inertial and transport, on std_logic, integer and vector drivers that other activations drive
-- at zero delay only (projected waveform rules, LRM 10.5.2.2); a signal forced for a while
-- whose driver keeps being assigned twice per activation.
library ieee;
use ieee.std_logic_1164.all;

entity redrive_after is
end entity;

architecture a of redrive_after is
  signal clk : std_logic := '0';
  signal s, t, f : std_logic := '0';
  signal n : integer := 0;
  signal v : std_logic_vector(3 downto 0) := "0000";
begin
  clkgen : process
  begin
    for i in 1 to 16 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;

  drv : process (clk)
    variable k : integer := 0;
  begin
    if rising_edge(clk) then
      k := k + 1;
      case k mod 4 is
        when 0 =>
          s <= '1';
          s <= '0' after 2 ns;
          t <= transport '1';
          t <= transport '0' after 2 ns;
          n <= k;
          n <= k + 100 after 3 ns;
          v <= "1010";
          v(0) <= '1' after 1 ns;
        when 1 =>
          s <= '1' after 2 ns;
          s <= '0';
          t <= transport '1' after 2 ns;
          t <= transport '0';
          n <= k after 3 ns;
          n <= k + 1;
          v(1) <= '1' after 1 ns;
          v <= "0101";
        when 2 =>
          s <= not s;
          s <= s;
          t <= '1';
          t <= '1';
          n <= n + 1;
          n <= n + 2;
          v <= v;
          v(2) <= not v(2);
        when others =>
          s <= '1';
          s <= '0';
          t <= not t;
          n <= 7;
          n <= 7;
          v(3) <= '1';
          v(3) <= '0';
      end case;
      f <= '1';
      if k mod 2 = 0 then
        f <= '0';
      end if;
    end if;
  end process;

  frc : process
  begin
    wait for 42 ns;
    f <= force 'X';
    wait for 30 ns;
    f <= release;
    wait;
  end process;

  watch : process
  begin
    wait on s, t, n, v, f;
    report "s=" & to_string(s) & " t=" & to_string(t) & " n=" & integer'image(n)
      & " v=" & to_string(v) & " f=" & to_string(f);
  end process;
end architecture;
