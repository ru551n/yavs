-- LRM 5.3.2.2: an unconstrained out port with a type conversion in the formal part takes its
-- index range from the type mark (7 downto 4), not from the actual (3 downto 0).
library ieee; use ieee.numeric_bit.all;
entity sub is port (o : out bit_vector); end entity;
architecture a of sub is
begin
  process begin
    report "RES o " & integer'image(o'left) & " " & integer'image(o'right);
    o <= (o'range => '1');
    o(o'left) <= '0';
    wait;
  end process;
end architecture;
library ieee; use ieee.numeric_bit.all;
entity top is end entity;
architecture a of top is
  subtype u4 is unsigned(7 downto 4);
  signal s : unsigned(3 downto 0);
begin
  u1 : entity work.sub port map (u4(o) => s);
  process begin wait for 1 ns; report "RES s " & to_string(s); wait; end process;
end architecture;
