-- Conditions elaboration evaluates (globally static calls of `numeric_std` whose arguments the
-- folds do not know themselves, `to_signed(0, 2)`) prune their branch alike in the subset checker
-- and the lowering (`W` replaced by a width): a register and a combinational process whose
-- pruned branch reads a signal nothing else reads join.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_elabcall_u is
  port (clk : in std_ulogic; x, z : in std_ulogic_vector(W - 1 downto 0);
        q, y : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_elabcall_u is
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      if (to_signed(0, 2) >= 4) then
        q <= z;
      else
        q <= not x;
      end if;
    end if;
  end process;
  comb : process (x, z)
  begin
    if to_unsigned(1, 3) < 0 then
      y <= z;
    elsif to_signed(-1, 2) = -1 then
      y <= x;
    else
      y <= z;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_elabcall is
end entity;
architecture t of cycle_elabcall is
  signal clk : std_ulogic := '0';
  signal x, z : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal q, y : std_ulogic_vector(W - 1 downto 0);
begin
  u : entity work.cycle_elabcall_u port map (clk, x, z, q, y);
  main : process
    variable s : unsigned(31 downto 0) := x"ACE1F00D";
  begin
    for k in 0 to 9 loop
      for b in x'range loop
        s := s(30 downto 0) & (s(31) xor s(21) xor s(1) xor s(0));
        x(b) <= s(0);
        z(b) <= s(3);
      end loop;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " " & to_hstring(q) & " " & to_hstring(y);
    end loop;
    wait;
  end process;
end architecture;
