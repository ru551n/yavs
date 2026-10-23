-- LRM 2019 5.2.3.1 / 2008 5.2.3.1: the range of INTEGER (it includes the symmetric 32-bit and 64-bit ranges).
entity top is end entity;
architecture a of top is
begin
  process begin
    report "RES low=" & integer'image(integer'low) & " high=" & integer'image(integer'high);
    wait;
  end process;
end architecture;
