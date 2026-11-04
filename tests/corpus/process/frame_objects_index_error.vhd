-- difftest: top=frame_objects_index_error exit=1 ref=hand
-- An index out of range of a composite variable in the frame stops at the same statement, after
-- many activations too, and the neighbours are not written. ref=hand: the expected text is yavs's
-- wording of the run-time error (the references fail on the same index with other words).
library ieee;
use ieee.std_logic_1164.all;

entity frame_objects_index_error is
end entity;

architecture a of frame_objects_index_error is
  type regs is array (0 to 3) of std_ulogic_vector(7 downto 0);
  signal clk : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  process (clk)
    variable b : regs := (others => x"11");
    variable v : regs := (others => x"22");
    variable a : regs := (others => x"33");
    variable i : integer := 0;
  begin
    if rising_edge(clk) then
      i := i + 1;
      if i = 2000 then
        report to_hstring(b(0)) & to_hstring(a(0));
        v(i - 1996) := x"44";
        report "not reached" severity failure;
      end if;
      v(i mod 4) := x"55";
    end if;
  end process;
end architecture;
