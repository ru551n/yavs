-- LRM 10.5.2.1: `release` of a whole record signal releases every scalar subelement, also
-- after one field was released on its own and while the record is assigned.
library ieee; use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  type rec is record a : std_logic; b : std_logic_vector(7 downto 0); n : integer; end record;
  signal r : rec := ('0', x"11", 1);
begin
  process begin
    r <= force ('1', x"a5", 7);
    wait for 1 ns;
    r.n <= 100;
    wait for 1 ns;
    r.n <= release;
    wait for 1 ns;
    r <= release;
    wait for 1 ns;
    report "RES released " & to_hstring(r.b) & " " & integer'image(r.n);
    wait;
  end process;
end architecture;
