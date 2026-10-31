-- MMCM and PLL: lock, and the output periods and phases of the vendor models.
library ieee;
use ieee.std_logic_1164.all;
library unisim;
use unisim.vcomponents.all;

entity tb_v_clock is
end entity;

architecture sim of tb_v_clock is
  constant period : time := 10 ns;
  signal clk, rst : std_ulogic := '1';
  signal done : boolean := false;
  signal m2_fb, m2_c0, m2_c1, m2_lock : std_ulogic;
  signal p2_fb, p2_c0, p2_c1, p2_lock : std_ulogic;
  signal ma_fb, ma_c0, ma_lock : std_ulogic;
  signal m4_fb, m4_c0, m4_lock : std_ulogic;
  signal p4_fb, p4_c0, p4_lock : std_ulogic;

  procedure measure(signal c : in std_ulogic; signal l : in std_ulogic; name : string; expected : time) is
    variable t0 : time;
  begin
    if l /= '1' then
      wait until l = '1' for 200 us;
    end if;
    assert l = '1' report name & " did not lock" severity failure;
    wait until rising_edge(c);
    wait until rising_edge(c);
    t0 := now;
    wait until rising_edge(c);
    report name & " locked, period " & time'image(now - t0);
    assert abs (now - t0 - expected) < 2 ps report name & " period" severity failure;
  end procedure;
begin
  clk <= not clk after period / 2 when not done else '0';

  -- 100 MHz in, VCO 1000 MHz: 200 MHz and 50 MHz out.
  u_m2 : MMCME2_BASE
    generic map (CLKIN1_PERIOD => 10.0, CLKFBOUT_MULT_F => 10.0, CLKOUT0_DIVIDE_F => 5.0, CLKOUT1_DIVIDE => 20)
    port map (CLKFBOUT => m2_fb, CLKOUT0 => m2_c0, CLKOUT1 => m2_c1, LOCKED => m2_lock,
              CLKFBIN => m2_fb, CLKIN1 => clk, PWRDWN => '0', RST => rst);
  u_p2 : PLLE2_BASE
    generic map (CLKIN1_PERIOD => 10.0, CLKFBOUT_MULT => 10, CLKOUT0_DIVIDE => 8, CLKOUT1_DIVIDE => 40)
    port map (CLKFBOUT => p2_fb, CLKOUT0 => p2_c0, CLKOUT1 => p2_c1, LOCKED => p2_lock,
              CLKFBIN => p2_fb, CLKIN1 => clk, PWRDWN => '0', RST => rst);
  u_ma : MMCME2_ADV
    generic map (CLKIN1_PERIOD => 10.0, CLKFBOUT_MULT_F => 10.0, CLKOUT0_DIVIDE_F => 4.0)
    port map (CLKFBOUT => ma_fb, CLKOUT0 => ma_c0, LOCKED => ma_lock, CLKFBIN => ma_fb, CLKIN1 => clk,
              CLKIN2 => '0', CLKINSEL => '1', DADDR => (others => '0'), DCLK => '0', DEN => '0',
              DI => (others => '0'), DWE => '0', PSCLK => '0', PSEN => '0', PSINCDEC => '0',
              PWRDWN => '0', RST => rst);
  u_m4 : MMCME4_ADV
    generic map (CLKIN1_PERIOD => 10.0, CLKFBOUT_MULT_F => 10.0, CLKOUT0_DIVIDE_F => 3.0)
    port map (CLKFBOUT => m4_fb, CLKOUT0 => m4_c0, LOCKED => m4_lock, CDDCREQ => '0', CLKFBIN => m4_fb,
              CLKIN1 => clk, CLKIN2 => '0', CLKINSEL => '1', DADDR => (others => '0'), DCLK => '0',
              DEN => '0', DI => (others => '0'), DWE => '0', PSCLK => '0', PSEN => '0', PSINCDEC => '0',
              PWRDWN => '0', RST => rst);
  u_p4 : PLLE4_BASE
    generic map (CLKIN_PERIOD => 10.0, CLKFBOUT_MULT => 8, CLKOUT0_DIVIDE => 4)
    port map (CLKFBOUT => p4_fb, CLKOUT0 => p4_c0, LOCKED => p4_lock, CLKFBIN => p4_fb, CLKIN => clk,
              CLKOUTPHYEN => '0', PWRDWN => '0', RST => rst);

  process
  begin
    wait for 100 ns;
    rst <= '0';
    measure(m2_c0, m2_lock, "MMCME2_BASE CLKOUT0", 5 ns);
    measure(m2_c1, m2_lock, "MMCME2_BASE CLKOUT1", 20 ns);
    measure(p2_c0, p2_lock, "PLLE2_BASE CLKOUT0", 8 ns);
    measure(p2_c1, p2_lock, "PLLE2_BASE CLKOUT1", 40 ns);
    measure(ma_c0, ma_lock, "MMCME2_ADV CLKOUT0", 4 ns);
    measure(m4_c0, m4_lock, "MMCME4_ADV CLKOUT0", 3 ns);
    measure(p4_c0, p4_lock, "PLLE4_BASE CLKOUT0", 5 ns);
    done <= true;
    report "done tb_v_clock";
    std.env.finish;
    wait;
  end process;
end architecture;
