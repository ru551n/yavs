-- Double data rate registers and delay elements.
library ieee;
use ieee.std_logic_1164.all;
library unisim;
use unisim.vcomponents.all;

entity tb_v_ddr is
end entity;

architecture sim of tb_v_ddr is
  signal clk, clk_n, refclk : std_ulogic := '0';
  signal done : boolean := false;
  signal rst : std_ulogic := '1';
  signal d : std_ulogic := '0';
  signal q1, q2, q1e, q2e : std_ulogic;
  signal d1, d2, q_o, q_oe : std_ulogic := '0';
  signal rdy, dly_o : std_ulogic;
begin
  clk <= not clk after 5 ns when not done else '0';
  clk_n <= not clk;
  refclk <= not refclk after 2.5 ns when not done else '0';

  u_iddr : IDDR
    generic map (DDR_CLK_EDGE => "SAME_EDGE_PIPELINED", SRTYPE => "SYNC")
    port map (Q1 => q1, Q2 => q2, C => clk, CE => '1', D => d, R => rst, S => '0');
  u_iddre1 : IDDRE1
    generic map (DDR_CLK_EDGE => "SAME_EDGE_PIPELINED")
    port map (Q1 => q1e, Q2 => q2e, C => clk, CB => clk_n, D => d, R => rst);
  u_oddr : ODDR
    generic map (DDR_CLK_EDGE => "SAME_EDGE", SRTYPE => "SYNC")
    port map (Q => q_o, C => clk, CE => '1', D1 => d1, D2 => d2, R => rst, S => '0');
  u_oddre1 : ODDRE1
    port map (Q => q_oe, C => clk, D1 => d1, D2 => d2, SR => rst);
  u_ctrl : IDELAYCTRL port map (RDY => rdy, REFCLK => refclk, RST => rst);
  u_dly : IDELAYE2
    generic map (IDELAY_TYPE => "FIXED", IDELAY_VALUE => 10, REFCLK_FREQUENCY => 200.0, DELAY_SRC => "IDATAIN")
    port map (DATAOUT => dly_o, C => '0', CE => '0', CINVCTRL => '0', CNTVALUEIN => (others => '0'), DATAIN => '0',
              IDATAIN => d, INC => '0', LD => '0', LDPIPEEN => '0', REGRST => '0');

  process
    variable t0 : time;
  begin
    wait for 72 ns;
    rst <= '0';
    wait until rdy = '1' for 2 us;
    assert rdy = '1' report "IDELAYCTRL ready" severity failure;

    -- Input: 1 before the rising edge, 0 before the falling edge.
    for k in 1 to 4 loop
      wait until falling_edge(clk);
      wait for 2 ns;
      d <= '1';
      wait until rising_edge(clk);
      wait for 2 ns;
      d <= '0';
    end loop;
    wait until rising_edge(clk);
    wait for 1 ns;
    report "IDDR q1 q2 " & to_string(q1) & to_string(q2) & " IDDRE1 " & to_string(q1e) & to_string(q2e);
    assert q1 = '1' and q2 = '0' report "IDDR" severity failure;
    assert q1e = '1' and q2e = '0' report "IDDRE1" severity failure;

    -- Output: 1 in the high phase, 0 in the low phase.
    d1 <= '1';
    d2 <= '0';
    wait until rising_edge(clk);
    wait until rising_edge(clk);
    wait for 1 ns;
    report "ODDR high " & to_string(q_o) & to_string(q_oe);
    assert q_o = '1' report "ODDR high phase" severity failure;
    wait until falling_edge(clk);
    wait for 1 ns;
    report "ODDR low " & to_string(q_o) & to_string(q_oe);
    assert q_o = '0' report "ODDR low phase" severity failure;

    -- Fixed delay: 10 taps of 78 ps at 200 MHz plus the data path offset of the model.
    wait until rising_edge(clk);
    d <= '1';
    t0 := now;
    wait until dly_o = '1' for 5 ns;
    report "IDELAYE2 delay " & time'image(now - t0);
    assert now - t0 = 1380 ps report "IDELAYE2 delay" severity failure;
    done <= true;
    report "done tb_v_ddr";
    std.env.finish;
    wait;
  end process;
end architecture;
