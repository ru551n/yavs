-- difftest: top=redrive_active
-- Assignments repeated in one activation where the activity of the signals is read
-- (`'active`, `'transaction`, `'last_active`, `'quiet`): a transaction that keeps the value
-- still makes the signal active, also when an earlier assignment of the run had another value
-- (one transaction with the last value, LRM 10.5.2.2), for a std_logic, an integer and a
-- vector with one of its elements.
library ieee;
use ieee.std_logic_1164.all;

entity redrive_active is
end entity;

architecture a of redrive_active is
  signal clk : std_logic := '0';
  signal s : std_logic := '0';
  signal n : integer := 0;
  signal v : std_logic_vector(3 downto 0) := "0000";
begin
  clkgen : process
  begin
    for i in 1 to 10 loop
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
      if k mod 3 /= 1 then
        s <= not s;
        s <= s;
        n <= n + 1;
        n <= n;
      else
        s <= s;
        s <= not s;
        n <= n;
        n <= n + k;
      end if;
      v <= not v;
      v(1) <= v(1);
      if k mod 2 = 0 then
        v <= v;
      end if;
    end if;
  end process;

  watch : process
  begin
    wait on s'transaction, n'transaction, v'transaction;
    report "s=" & to_string(s) & " act " & boolean'image(s'active) & " ev " & boolean'image(s'event)
      & " q " & boolean'image(s'quiet) & " la " & time'image(s'last_active)
      & " n=" & integer'image(n) & " act " & boolean'image(n'active) & " ev " & boolean'image(n'event)
      & " v=" & to_string(v) & " act " & boolean'image(v'active) & " ev " & boolean'image(v'event)
      & " v1 " & boolean'image(v(1)'active);
  end process;
end architecture;
