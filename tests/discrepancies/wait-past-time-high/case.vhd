-- LRM 10.2, 14.7.5: a wait timeout and a transaction whose time is past TIME'HIGH. The
-- LRM does not say; they never mature and the simulation ends at TIME'HIGH.
entity top is end entity;
architecture a of top is
  signal s : bit;
begin
  process
  begin
    wait for time'high - 1 ns;
    s <= '1' after 2 ns;
    report "RES assigned";
    wait for 2 ns;
    report "RES resumed past TIME'HIGH";
    wait;
  end process;
end architecture;
