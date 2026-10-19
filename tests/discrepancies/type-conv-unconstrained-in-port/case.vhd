-- LRM 5.3.2.2: an unconstrained in port associated with a type conversion whose type mark is
-- a constrained subtype takes its index range from the type mark (1 to 4), not from the actual.
library ieee; use ieee.numeric_bit.all;
entity sub is port (i : in bit_vector); end entity;
architecture a of sub is
begin
  process begin
    report "RES i " & integer'image(i'left) & " " & integer'image(i'right) & " " & to_string(i);
    wait;
  end process;
end architecture;
library ieee; use ieee.numeric_bit.all;
entity top is end entity;
architecture a of top is
  subtype w4 is bit_vector(1 to 4);
  signal u : unsigned(3 downto 0) := "1100";
begin
  u1 : entity work.sub port map (i => w4(u));
end architecture;
