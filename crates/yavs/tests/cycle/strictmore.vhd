-- Reason variants of `--cycle-strict` beyond one per code: each process below holds one, named in
-- `cycle_strict_reason_variants_are_triggered` with the text it gives.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_more is
  port (clk, rst, a, b : in std_ulogic; v : in std_ulogic_vector(3 downto 0); q : out std_ulogic_vector(0 to 15));
end entity;
architecture rtl of d_more is
  type ptr is access integer;
  signal s_null : std_logic register := '0';
  signal n_out : natural;
begin
  reset_reads_clock : process (clk)
  begin
    if clk = '1' then
      q(0) <= '0';
    elsif rising_edge(clk) then
      q(0) <= a;
    end if;
  end process;
  reset_no_signal : process (clk)
  begin
    if true then
      q(1) <= '0';
    elsif rising_edge(clk) then
      q(1) <= a;
    end if;
  end process;
  next_outer : process (clk)
    variable n : natural;
  begin
    if rising_edge(clk) then
      n := 0;
      outer : for i in 0 to 3 loop
        for j in 0 to 3 loop
          next outer when v(j) = '1';
          n := n + 1;
        end loop;
      end loop;
      q(2) <= '1' when n > 3 else '0';
    end if;
  end process;
  no_exit : process (a)
    variable n : natural;
  begin
    n := 0;
    loop
      n := n + 1;
    end loop;
    q(3) <= a;
  end process;
  big_loops : process (clk)
    variable n : natural;
  begin
    if rising_edge(clk) then
      n := 0;
      for i in 0 to 299 loop
        for j in 0 to 299 loop
          n := (n + i * j) mod 7;
        end loop;
      end loop;
      q(4) <= '1' when n = 3 else '0';
    end if;
  end process;
  uses_access : process (clk)
    variable p : ptr;
  begin
    if rising_edge(clk) then
      p := new integer'(1);
      q(5) <= a;
    end if;
  end process;
  guarded_null : block (a = '1')
  begin
    nul : process (b)
    begin
      if b = '1' then
        s_null <= null;
      else
        s_null <= '1';
      end if;
    end process;
  end block;
  q(6) <= s_null;
  dyn_loop : process (a, v)
    variable n : natural;
  begin
    n := 0;
    for i in 0 to to_integer(unsigned(v)) loop
      n := n + 1;
    end loop;
    q(7) <= '1' when n > 2 else a;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity t_more is
end entity;
architecture tb of t_more is
  signal clk, rst, a, b : std_ulogic := '0';
  signal v : std_ulogic_vector(3 downto 0) := "0000";
  signal q : std_ulogic_vector(0 to 15);
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_more port map (clk, rst, a, b, v, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;
