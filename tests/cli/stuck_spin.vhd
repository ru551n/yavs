-- A process without a wait statement that never ends: the run spins inside one activation.
entity stuck_spin is end entity;
architecture a of stuck_spin is
  signal a : bit := '0';
begin
  p : process
  begin
    a <= not a;
  end process;
end architecture;
