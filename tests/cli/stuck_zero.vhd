-- A process that waits for 0 ns in a loop.
entity stuck_zero is end entity;
architecture a of stuck_zero is
begin
  p : process
  begin
    wait for 0 ns;
  end process;
end architecture;
