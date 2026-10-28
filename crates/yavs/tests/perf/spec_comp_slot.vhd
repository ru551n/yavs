-- A process with an array-of-composite variable (a 24-byte slot, no descriptor in it) followed by a
-- variable passed by reference: the specialised copy (YAVS_SPEC_AFTER) must see the second
-- variable's address escape and not fold its bytes (#856).
library ieee;
use ieee.std_logic_1164.all;
entity spec_comp_slot is generic (W : natural := 4); end;
architecture a of spec_comp_slot is
  type arr is array (0 to 1) of std_logic_vector(W - 1 downto 0);
  signal clk : std_logic := '0';
  signal n : natural := 0;
  procedure inc(x : inout integer) is begin x := x + 1; end;
begin
  clk <= not clk after 5 ns when n < 20;
  process (clk)
    variable m : arr;
    variable k : integer := 7;
  begin
    if rising_edge(clk) then
      inc(k);
      m(0) := (others => '1');
      n <= n + 1;
      if n = 19 then report "k=" & integer'image(k); end if;
    end if;
  end process;
end;
