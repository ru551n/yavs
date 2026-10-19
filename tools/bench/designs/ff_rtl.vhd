-- Many flip-flops: a clocked pipeline of LANES counters, each with a few gates of logic.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity bench_ff_rtl is
  generic (LANES : positive := 64; CYCLES : positive := 150);
end entity;

architecture sim of bench_ff_rtl is
  signal clk : std_logic := '0';
  type word_array is array (0 to LANES - 1) of unsigned(7 downto 0);
  signal cnt : word_array := (others => (others => '0'));
  signal par : std_logic_vector(LANES - 1 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  lanes_g : for i in 0 to LANES - 1 generate
    process (clk)
    begin
      if rising_edge(clk) then
        if i = 0 then
          cnt(i) <= cnt(i) + 1;
        else
          cnt(i) <= cnt(i) + cnt(i - 1) + 1;
        end if;
        par(i) <= cnt(i)(0) xor cnt(i)(7);
      end if;
    end process;
  end generate;

  process
  begin
    for c in 1 to CYCLES loop
      wait until rising_edge(clk);
    end loop;
    done <= true;
    report "ff_rtl cnt(last)=" & integer'image(to_integer(cnt(LANES - 1)));
    wait;
  end process;
end architecture;
