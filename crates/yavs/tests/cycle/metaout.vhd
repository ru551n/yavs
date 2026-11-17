-- Metavalues the testbench must see (the region gives back '0' and '1' only): `drv` assigns an
-- 'X' literal to `data`, which `chk` reads; `copy` passes on the 'Z' that `con`'s driver
-- (outside: it uses 'Z') may give. Both stay in the event kernel. `ix` assigns 'X' to a signal
-- only members read, and `copy2` copies a signal without 'Z': both are members.
library ieee; use ieee.std_logic_1164.all;
entity cycle_metaout is
  generic (W : positive := 8);
end;
architecture a of cycle_metaout is
  signal clk, en, d, con, pin, pin2, valid : std_logic := '0';
  signal data, xi, y : std_logic_vector(W - 1 downto 0) := (others => '0');
  signal mask : std_logic_vector(W - 1 downto 0) := (others => '0');
  signal cnt : integer range 0 to 7 := 0;
begin
  clk <= not clk after 5 ns when now < 100 ns else clk;
  con <= d when en = '1' else 'Z';
  copy : process (con) begin pin <= con; end process;
  copy2 : process (d) begin pin2 <= d; end process;
  drv : process (clk) begin
    if rising_edge(clk) then
      cnt <= (cnt + 1) mod 8;
      valid <= '1' when cnt < 4 else '0';
      if cnt >= 4 then
        data <= (others => 'X');
      elsif cnt mod 2 = 1 then
        data <= (others => '1');
      else
        data <= (others => '0');
      end if;
    end if;
  end process;
  ix : process (clk) begin
    if rising_edge(clk) then
      xi <= (others => 'X');
    end if;
  end process;
  iy : process (xi, mask) begin y <= xi and mask; end process;
  stim : process begin
    wait for 12 ns; report "pin=" & std_logic'image(pin) & " pin2=" & std_logic'image(pin2);
    en <= '1'; d <= '1'; wait for 10 ns;
    report "pin=" & std_logic'image(pin) & " pin2=" & std_logic'image(pin2);
    en <= '0'; wait for 10 ns;
    report "pin=" & std_logic'image(pin) & " pin2=" & std_logic'image(pin2);
    wait;
  end process;
  chk : process begin
    wait until falling_edge(clk) and valid = '0' and now > 30 ns;
    report "invalid data " & to_string(data) & " x=" & boolean'image(is_x(data)) & " y=" & to_string(y);
    wait;
  end process;
end;
