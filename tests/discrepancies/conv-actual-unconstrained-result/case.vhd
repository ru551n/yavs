-- LRM 5.3.2.2: a type conversion in the actual part of an unconstrained in port must define
-- the formal's index range; `bit_vector(u)` does not, so the association is an error.
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
  signal u : unsigned(3 downto 0) := "1100";
begin
  u1 : entity work.sub port map (i => bit_vector(u));
end architecture;
