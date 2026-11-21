-- Cycle mode with `--wave`: signals only the region reads, of width W (a register, logic on it,
-- logic on inputs that change between edges (`din2` reaches no output), a record, an array, a memory of logic and one of
-- integers, a register without a start value), recorded per time step as in the normal mode.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_waves_dut is
  port (
    clk, rst : in std_ulogic;
    din, din2 : in std_ulogic_vector(W - 1 downto 0);
    dout : out std_ulogic_vector(W - 1 downto 0)
  );
end entity;
architecture a of cycle_waves_dut is
  type rec_t is record
    f : std_ulogic;
    v : unsigned(W - 1 downto 0);
    n : integer range 0 to 999;
  end record;
  type arr_t is array (0 to 3) of std_ulogic_vector(7 downto 0);
  type ram_t is array (0 to 1023) of std_ulogic_vector(7 downto 0);
  type iram_t is array (0 to 127) of integer;
  signal q, c, mix, mix2 : unsigned(W - 1 downto 0);
  signal r : rec_t;
  signal arr : arr_t;
  signal ram : ram_t := (others => (others => '0'));
  signal iram : iram_t := (others => 0);
  signal rd : std_ulogic_vector(7 downto 0) := (others => '0');
  signal ird : integer := 0;
  signal cnt : integer range 0 to 1023;
  signal noinit : std_ulogic_vector(W - 1 downto 0);
begin
  regs : process (clk, rst)
  begin
    if rst = '1' then
      q <= (others => '0');
      r <= ('0', (others => '0'), 0);
      arr <= (others => x"00");
      cnt <= 0;
    elsif rising_edge(clk) then
      q <= q + 1 + unsigned(din);
      r.f <= q(0);
      r.v <= c;
      r.n <= (r.n + 3) mod 1000;
      arr(cnt mod 4) <= std_ulogic_vector(to_unsigned(cnt mod 256, 8));
      cnt <= (cnt + 1) mod 1024;
    end if;
  end process;

  mem : process (clk)
  begin
    if rising_edge(clk) then
      ram(cnt) <= std_ulogic_vector(to_unsigned((cnt * 5) mod 256, 8));
      rd <= ram((cnt + 1023) mod 1024);
      iram(cnt mod 128) <= cnt * 7;
      ird <= iram((cnt + 127) mod 128);
      if cnt > 3 then
        noinit <= din;
      end if;
    end if;
  end process;

  c <= q xor shift_right(q, 1);
  mix <= unsigned(din) and not q;
  mix2 <= unsigned(din2) xor q;
  dout <= std_ulogic_vector(c) xor rd(0) xor std_ulogic_vector(mix)
          xor std_ulogic(to_unsigned(ird mod 2, 1)(0));
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_waves is
end entity;
architecture a of cycle_waves is
  signal clk, rst : std_ulogic := '0';
  signal din, din2, dout : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  dut : entity work.cycle_waves_dut port map (clk, rst, din, din2, dout);
  main : process
  begin
    rst <= '1';
    wait for 3 ns;
    rst <= '0';
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 2 ns;
      -- Between edges: `mix`, `mix2` and `dout` see these before the next edge.
      for k in din'range loop
        if (i + k) mod 3 = 0 then
          din(k) <= '1';
        else
          din(k) <= '0';
        end if;
      end loop;
      wait for 1 ns;
      din2 <= not din;
      wait for 2 ns;
    end loop;
    report "dout=" & to_hstring(dout);
    std.env.finish;
  end process;
end architecture;
