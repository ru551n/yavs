-- A process that waits only on a signal it assigns.
entity stuck_self is end entity;
architecture a of stuck_self is
  signal a : bit := '0';
begin
  p : process
  begin
    a <= not a;
    wait on a;
  end process;
end architecture;
