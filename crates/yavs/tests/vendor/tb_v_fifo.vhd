-- Hard FIFOs and FIFO macros: reset sequence, write, read, flags, one clock and two.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
library unisim;
use unisim.vcomponents.all;
library unimacro;
use unimacro.vcomponents.all;

entity tb_v_fifo is
end entity;

architecture sim of tb_v_fifo is
  signal clk, rclk : std_ulogic := '0';
  signal done : boolean := false;
  signal rst : std_ulogic := '1';
  signal wr, rd : std_ulogic := '0';
  signal din : std_logic_vector(63 downto 0) := (others => '0');
  signal q_36e1, q_36e2 : std_logic_vector(63 downto 0);
  signal q_18e1 : std_logic_vector(31 downto 0);
  signal q_sync, q_dual : std_logic_vector(31 downto 0);
  signal e_36e1, e_36e2, e_18e1, e_sync, e_dual : std_ulogic;
  signal f_36e1, f_36e2 : std_ulogic;
  signal wrb, rdb : std_ulogic;
begin
  clk <= not clk after 5 ns when not done else '0';
  rclk <= not rclk after 7 ns when not done else '0';

  u_36e1 : FIFO36E1
    generic map (DATA_WIDTH => 72, FIFO_MODE => "FIFO36_72", DO_REG => 1, EN_SYN => false, FIRST_WORD_FALL_THROUGH => true)
    port map (DO => q_36e1, EMPTY => e_36e1, FULL => f_36e1, DI => din, DIP => X"00", INJECTDBITERR => '0',
              INJECTSBITERR => '0', RDCLK => rclk, RDEN => rd, REGCE => '1', RST => rst, RSTREG => '0', WRCLK => clk, WREN => wr);
  u_18e1 : FIFO18E1
    generic map (DATA_WIDTH => 36, FIFO_MODE => "FIFO18_36", DO_REG => 1, EN_SYN => false, FIRST_WORD_FALL_THROUGH => true)
    port map (DO => q_18e1, EMPTY => e_18e1, DI => din(31 downto 0), DIP => X"0", RDCLK => rclk, RDEN => rd,
              REGCE => '1', RST => rst, RSTREG => '0', WRCLK => clk, WREN => wr);
  u_36e2 : FIFO36E2
    generic map (READ_WIDTH => 72, WRITE_WIDTH => 72, FIRST_WORD_FALL_THROUGH => "TRUE", REGISTER_MODE => "REGISTERED")
    port map (DOUT => q_36e2, EMPTY => e_36e2, FULL => f_36e2, RDRSTBUSY => rdb, WRRSTBUSY => wrb, CASDIN => (others => '0'),
              CASDINP => X"00", CASDOMUX => '0', CASDOMUXEN => '1', CASNXTRDEN => '0', CASOREGIMUX => '0',
              CASOREGIMUXEN => '1', CASPRVEMPTY => '0', DIN => din, DINP => X"00", INJECTDBITERR => '0',
              INJECTSBITERR => '0', RDCLK => rclk, RDEN => rd, REGCE => '1', RST => rst, RSTREG => '0', SLEEP => '0',
              WRCLK => clk, WREN => wr);
  u_sync : FIFO_SYNC_MACRO
    generic map (DEVICE => "7SERIES", DATA_WIDTH => 32, FIFO_SIZE => "36Kb", DO_REG => 0)
    port map (DO => q_sync, EMPTY => e_sync, CLK => clk, DI => din(31 downto 0), RDEN => rd, RST => rst, WREN => wr);
  u_dual : FIFO_DUALCLOCK_MACRO
    generic map (DEVICE => "7SERIES", DATA_WIDTH => 32, FIFO_SIZE => "36Kb", FIRST_WORD_FALL_THROUGH => true)
    port map (DO => q_dual, EMPTY => e_dual, DI => din(31 downto 0), RDCLK => rclk, RDEN => rd, RST => rst, WRCLK => clk, WREN => wr);

  process
    procedure tick(n : positive := 1) is
    begin
      for k in 1 to n loop
        wait until rising_edge(clk);
      end loop;
      wait for 1 ns;
    end procedure;
  begin
    tick(8);
    rst <= '0';
    tick(40);
    report "after reset empty " & to_string(e_36e1) & to_string(e_18e1) & to_string(e_36e2) & to_string(e_sync) & to_string(e_dual)
      & " full " & to_string(f_36e1) & to_string(f_36e2);
    assert e_36e1 = '1' and e_18e1 = '1' and e_36e2 = '1' and e_sync = '1' and e_dual = '1' report "empty after reset" severity failure;

    wr <= '1';
    for k in 1 to 20 loop
      din <= std_logic_vector(to_unsigned(k, 32)) & std_logic_vector(to_unsigned(k * 3, 32));
      tick;
    end loop;
    wr <= '0';
    tick(40);
    assert e_36e1 = '0' and e_18e1 = '0' and e_36e2 = '0' and e_dual = '0' report "not empty after writes" severity failure;
    report "first word " & to_hstring(q_36e1) & " " & to_hstring(q_18e1) & " " & to_hstring(q_36e2) & " " & to_hstring(q_dual);

    for k in 1 to 20 loop
      wait until falling_edge(rclk);
      assert unsigned(q_36e1(31 downto 0)) = k * 3 report "FIFO36E1 word " & integer'image(k) & " is " & to_hstring(q_36e1) severity failure;
      assert unsigned(q_18e1) = k * 3 report "FIFO18E1 word " & integer'image(k) & " is " & to_hstring(q_18e1) severity failure;
      assert unsigned(q_36e2(31 downto 0)) = k * 3 report "FIFO36E2 word " & integer'image(k) & " is " & to_hstring(q_36e2) severity failure;
      assert unsigned(q_dual) = k * 3 report "FIFO_DUALCLOCK_MACRO word " & integer'image(k) & " is " & to_hstring(q_dual) severity failure;
      rd <= '1';
      wait until rising_edge(rclk);
      wait for 1 ns;
      rd <= '0';
    end loop;
    tick(10);
    report "drained empty " & to_string(e_36e1) & to_string(e_18e1) & to_string(e_36e2) & to_string(e_dual);
    assert e_36e1 = '1' and e_18e1 = '1' and e_36e2 = '1' and e_dual = '1' report "empty after reads" severity failure;
    done <= true;
    report "done tb_v_fifo";
    std.env.finish;
    wait;
  end process;
end architecture;
