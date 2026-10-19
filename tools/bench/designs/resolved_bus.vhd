-- Resolved bus: DRIVERS processes drive a std_logic_vector, all but one with 'Z'.
library ieee;
use ieee.std_logic_1164.all;

entity bench_resolved_bus is
  generic (DRIVERS : positive := 16; ROUNDS : positive := 15000);
end entity;

architecture sim of bench_resolved_bus is
  signal bus_s : std_logic_vector(31 downto 0);
  signal sel   : natural range 0 to DRIVERS - 1 := 0;
  signal done  : boolean := false;
begin
  drv : for d in 0 to DRIVERS - 1 generate
    process (sel)
      variable x : std_logic_vector(31 downto 0);
    begin
      if sel = d then
        x := (others => '0');
        x(d mod 32) := '1';
        bus_s <= x;
      else
        bus_s <= (others => 'Z');
      end if;
    end process;
  end generate;

  process
    variable ones : natural := 0;
  begin
    for r in 1 to ROUNDS loop
      sel <= r mod DRIVERS;
      wait for 1 ns;
      for b in bus_s'range loop
        if bus_s(b) = '1' then
          ones := ones + 1;
        end if;
      end loop;
    end loop;
    assert ones = ROUNDS report "bus value mismatch" severity failure;
    report "resolved_bus ones=" & integer'image(ones);
    done <= true;
    wait;
  end process;
end architecture;
