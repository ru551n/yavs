-- LRM 14.7.5.2: does S'transaction toggle in the first cycle of the simulation?
entity top is end entity;
architecture a of top is
  signal n : natural := 1;
  signal tr : bit;
begin
  tr <= n'transaction;
  drv : process begin wait for 1 ns; n <= 5; wait; end process;
  mon : process (tr) begin report "RES tr " & bit'image(tr) & " at " & integer'image(now / 1 ns) & " ns"; end process;
end architecture;
