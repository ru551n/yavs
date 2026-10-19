-- difftest: top=wait_past_time_high std=2008
-- A wait timeout or a transaction past TIME'HIGH never matures: the processes stay
-- suspended and the simulation ends at TIME'HIGH without an error.
entity wait_past_time_high is end;
architecture t of wait_past_time_high is
  signal s : bit;
begin
  waiter: process
  begin
    wait for time'high - 1 ns;
    report "near TIME'HIGH";
    wait for 2 ns;
    report "resumed past TIME'HIGH" severity failure;
    wait;
  end process;

  driver: process
  begin
    wait for time'high - 2 ns;
    s <= '1' after 3 ns;
    s <= transport '1' after 4 ns;
    report "assigned past TIME'HIGH";
    wait on s;
    report "s changed past TIME'HIGH" severity failure;
    wait;
  end process;
end;
