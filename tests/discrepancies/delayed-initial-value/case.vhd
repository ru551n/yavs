-- LRM 14.7.5.2: the value of S'delayed(T) before T has passed is the initial value of S.
entity top is end entity;
architecture a of top is
  signal n : natural := 7;
  signal d : natural;
begin
  d <= n'delayed(5 ns);
  process begin
    wait for 1 ns;
    report "RES d=" & integer'image(d);
    wait;
  end process;
end architecture;
