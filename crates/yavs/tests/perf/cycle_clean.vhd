-- A81: a small datapath whose values are '0' and '1' once its reset has run (plain `--cycle`,
-- `YAVS_CYCLE_CLEAN=on`): its members run their plane-0 bodies, the nine-valued ones only while
-- the reset makes the values clean. The memory holds 'U' where it was never written, and is
-- read only where it was; a variable holds its value between edges.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_clean is end entity;
architecture tb of cycle_clean is
  signal clk, rst : std_ulogic := '1';
  signal pc, acc, nxt, rd : unsigned(15 downto 0);
  signal op : std_ulogic_vector(1 downto 0);
  type mem_t is array (0 to 511) of std_ulogic_vector(15 downto 0);
  signal mem : mem_t;
  signal done : boolean := false;
begin
  process (clk, rst) begin
    if rst = '1' then
      pc <= (others => '0');
      acc <= x"0001";
    elsif rising_edge(clk) then
      pc <= pc + 1;
      acc <= nxt;
    end if;
  end process;
  op <= std_ulogic_vector(pc(1 downto 0));
  process (op, acc, pc, rd) begin
    case op is
      when "00" => nxt <= acc + pc;
      when "01" => nxt <= acc xor rd;
      when "10" => nxt <= shift_left(acc, 1) or shift_right(acc, 15);
      when others => nxt <= acc - 3;
    end case;
  end process;
  process (clk)
    variable last : std_ulogic_vector(15 downto 0) := (others => '0');
  begin
    if rising_edge(clk) then
      mem(to_integer(pc(7 downto 0))) <= std_ulogic_vector(acc) xor last;
      last := std_ulogic_vector(acc);
      if pc > 1 then
        rd <= unsigned(mem(to_integer(pc(7 downto 0) - 1)));
      else
        rd <= (others => '0');
      end if;
    end if;
  end process;
  process begin
    wait for 5 ns;
    rst <= '0';
    for i in 1 to 2000 loop
      clk <= '0'; wait for 5 ns; clk <= '1'; wait for 5 ns;
    end loop;
    report "acc=" & to_hstring(acc);
    wait;
  end process;
end architecture;
