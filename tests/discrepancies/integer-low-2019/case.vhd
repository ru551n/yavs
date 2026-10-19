-- LRM 2019 5.2.3.1 / 2008 5.2.3.1: the minimum of INTEGER.
entity top is end entity;
architecture a of top is
begin
  process begin
    report "RES low=" & integer'image(integer'low) & " high=" & integer'image(integer'high);
    wait;
  end process;
end architecture;
