-- Cycle mode gate design: a request and acknowledge handshake between two unrelated clocks (10 ns and
-- 7 ns, so their edges coincide every 70 ns). Each side has a two-flop synchroniser; the data word is
-- captured in the receiving domain when the synchronised request toggles. A monitor in each domain
-- prints its registers at every rising edge of its clock and 1 ns after it.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_handshake is
end entity;
architecture a of cycle_handshake is
  signal clka, clkb : std_ulogic := '0';
  signal send : std_ulogic := '0';
  signal data_a : unsigned(7 downto 0) := x"00";
  signal req_t, ack_t : std_ulogic := '0';
  signal req_s1, req_s2, req_seen : std_ulogic := '0';
  signal ack_s1, ack_s2 : std_ulogic := '0';
  signal data_b : unsigned(7 downto 0) := x"00";
  signal got : unsigned(3 downto 0) := (others => '0');
  signal busy_a : std_ulogic := '0';
  signal sent : unsigned(3 downto 0) := (others => '0');
  impure function snap_a return string is
  begin
    return "send " & to_string(send) & " busy " & to_string(busy_a) & " req " & to_string(req_t) & " ack_s2 " & to_string(ack_s2)
      & " data " & to_string(to_integer(data_a)) & " sent " & to_string(to_integer(sent));
  end function;
  impure function snap_b return string is
  begin
    return "s1 " & to_string(req_s1) & " s2 " & to_string(req_s2) & " seen " & to_string(req_seen) & " ack " & to_string(ack_t)
      & " data " & to_string(to_integer(data_b)) & " got " & to_string(to_integer(got));
  end function;
begin
  clka <= not clka after 5 ns;
  clkb <= not clkb after 3500 ps;

  side_a : process (clka)
  begin
    if rising_edge(clka) then
      ack_s1 <= ack_t;
      ack_s2 <= ack_s1;
      if busy_a = '0' then
        if send = '1' then
          data_a <= data_a + 37;
          req_t <= not req_t;
          busy_a <= '1';
          sent <= sent + 1;
        end if;
      elsif ack_s2 = req_t then
        busy_a <= '0';
      end if;
    end if;
  end process;

  side_b : process (clkb)
  begin
    if rising_edge(clkb) then
      req_s1 <= req_t;
      req_s2 <= req_s1;
      req_seen <= req_s2;
      if req_s2 /= req_seen then
        data_b <= data_a;
        got <= got + 1;
        ack_t <= req_s2;
      end if;
    end if;
  end process;

  -- two waits each: they stay outside the region; they read the values before the edge, then after it
  mon_a : process
  begin
    wait until rising_edge(clka);
    report "A pre  " & snap_a;
    wait for 1 ns;
    report "A post " & snap_a;
  end process;

  mon_b : process
  begin
    wait until rising_edge(clkb);
    report "B pre  " & snap_b;
    wait for 1 ns;
    report "B post " & snap_b;
  end process;

  stim : process
  begin
    for i in 0 to 59 loop
      wait until falling_edge(clka);
      send <= '1' when i mod 4 /= 3 and i /= 40 else '0';
    end loop;
    wait for 100 ns;
    std.env.stop;
  end process;
end architecture;
