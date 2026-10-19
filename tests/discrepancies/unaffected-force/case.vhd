-- LRM 2019 10.5.2.1: `force unaffected` forces nothing.
entity top is end entity;
architecture a of top is
  signal s : integer := 4;
begin
  process begin
    s <= force unaffected;
    wait for 1 ns;
    report "RES s=" & integer'image(s);
    wait;
  end process;
end architecture;
