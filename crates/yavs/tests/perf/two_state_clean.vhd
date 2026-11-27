-- `--two-state` below the top: signals that only two-state processes drive (a counter, its
-- copy, both resolved `unsigned` with one driver, a record) are read without a map (`ops`).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity two_state_clean_dut is
  port (clk : in std_ulogic; sum : out natural);
end entity;

architecture rtl of two_state_clean_dut is
  type rec_t is record
    a : std_ulogic_vector(31 downto 0);
    b : std_ulogic;
  end record;
  signal cnt, copy : unsigned(31 downto 0) := (others => '0');
  signal r : rec_t := ((others => '0'), '0');
begin
  process (clk)
    variable s : natural := 0;
  begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
      copy <= cnt;
      r <= (std_ulogic_vector(copy), not r.b);
      for i in 0 to 7 loop
        if cnt(i) = '1' then s := s + 1; end if;
        if copy(i) = '1' then s := s + 1; end if;
        if r.a(i) = '1' then s := s + 1; end if;
      end loop;
      if r.b = '1' then s := s + 1; end if;
      sum <= s;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity two_state_clean is
end entity;

architecture tb of two_state_clean is
  signal clk : std_ulogic := '0';
  signal sum : natural;
begin
  dut : entity work.two_state_clean_dut port map (clk, sum);
  process
  begin
    for i in 1 to 200 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
