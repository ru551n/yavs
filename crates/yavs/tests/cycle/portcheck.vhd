-- A range check of a port's expression actual in the region (LRM 6.5.6.3): its message names the
-- port, as the normal mode's does, and points at the actual.
entity cycle_portcheck_mon is
  generic (lo, hi : integer);
  port (n : in integer range lo to hi := lo; clk : in bit; q : out integer);
end;
architecture a of cycle_portcheck_mon is
begin
  process (clk) begin
    if clk'event and clk = '1' then
      q <= n;
    end if;
  end process;
end;
entity cycle_portcheck is end;
architecture a of cycle_portcheck is
  signal x : integer := 1;
  signal clk : bit;
  signal q : integer;
begin
  m : entity work.cycle_portcheck_mon generic map (0, 3) port map (n => x * 2, clk => clk, q => q);
  process begin
    clk <= '1'; wait for 1 ns; clk <= '0'; wait for 1 ns;
    x <= 2;
    clk <= '1'; wait for 1 ns;
    wait;
  end process;
end;
