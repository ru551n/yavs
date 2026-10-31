-- Buffers, registers, LUTs, carry and shift registers from the vendor primitive library.
library ieee;
use ieee.std_logic_1164.all;
library unisim;
use unisim.vcomponents.all;

entity tb_v_basic is
end entity;

architecture sim of tb_v_basic is
  signal clk, clk_g, clk_ce, ce : std_ulogic := '0';
  signal clk_mux, sel            : std_ulogic := '0';
  signal d, q_r, q_c             : std_ulogic := '0';
  signal rst                     : std_ulogic := '1';
  signal i, ib, o_ds, obuf_o, ibuf_o : std_ulogic := '0';
  signal p, n                    : std_ulogic;
  signal io_pad                  : std_logic := 'Z';
  signal io_i, io_t, io_o        : std_ulogic := '0';
  signal lut_in                  : std_logic_vector(5 downto 0) := (others => '0');
  signal lut5, lut6, mux7        : std_ulogic;
  signal co, sum                 : std_logic_vector(3 downto 0);
  signal srl_q, srl32_q, srl32_c : std_ulogic;
  signal spo, dpo, ram_we        : std_ulogic := '0';
  signal done                    : boolean := false;
begin
  clk <= not clk after 5 ns when not done else '0';

  u_bufg : BUFG port map (O => clk_g, I => clk);
  u_bufgce : BUFGCE port map (O => clk_ce, CE => ce, I => clk);
  u_ctrl : BUFGCTRL port map (O => clk_mux, CE0 => '1', CE1 => '1', I0 => clk, I1 => '0',
                              IGNORE0 => '0', IGNORE1 => '0', S0 => not sel, S1 => sel);
  u_fdre : FDRE port map (Q => q_r, C => clk_g, CE => '1', D => d, R => rst);
  u_fdce : FDCE port map (Q => q_c, C => clk_g, CE => '1', D => d, CLR => rst);
  u_ibuf : IBUF port map (O => ibuf_o, I => i);
  u_obuf : OBUF port map (O => obuf_o, I => i);
  u_ibufds : IBUFDS port map (O => o_ds, I => i, IB => ib);
  u_obufds : OBUFDS port map (O => p, OB => n, I => i);
  u_iobuf : IOBUF port map (O => io_o, IO => io_pad, I => io_i, T => io_t);
  u_lut : LUT6_2 generic map (INIT => X"8000000000000001")
    port map (O5 => lut5, O6 => lut6, I0 => lut_in(0), I1 => lut_in(1), I2 => lut_in(2),
              I3 => lut_in(3), I4 => lut_in(4), I5 => lut_in(5));
  u_mux : MUXF7 port map (O => mux7, I0 => lut6, I1 => lut5, S => lut_in(5));
  u_carry : CARRY4 port map (CO => co, O => sum, CI => '0', CYINIT => '1', DI => "0101", S => "1010");
  u_srl : SRL16E generic map (INIT => X"0000")
    port map (Q => srl_q, A0 => '1', A1 => '1', A2 => '0', A3 => '0', CE => '1', CLK => clk_g, D => d);
  u_srl32 : SRLC32E generic map (INIT => X"00000000")
    port map (Q => srl32_q, Q31 => srl32_c, A => "00001", CE => '1', CLK => clk_g, D => d);
  u_ram : RAM32X1D generic map (INIT => X"00000000")
    port map (DPO => dpo, SPO => spo, A0 => '1', A1 => '0', A2 => '0', A3 => '0', A4 => '0', D => d,
              DPRA0 => '1', DPRA1 => '0', DPRA2 => '0', DPRA3 => '0', DPRA4 => '0', WCLK => clk_g, WE => ram_we);

  process
  begin
    wait for 1 ns;
    assert clk_g = clk report "BUFG" severity failure;
    wait until rising_edge(clk);
    wait for 1 ns;
    -- BUFGCTRL holds its output low until the startup sequence of the model has passed.
    wait for 400 ns;
    wait until rising_edge(clk);
    wait for 1 ns;
    assert clk_g = '1' and clk_mux = '1' report "BUFG follows, BUFGCTRL selects I0" severity failure;
    assert clk_ce = '0' report "BUFGCE stopped" severity failure;
    ce <= '1';
    wait until rising_edge(clk);
    wait for 1 ns;
    assert clk_ce = '1' report "BUFGCE runs" severity failure;
    report "clock buffers ok";

    rst <= '0'; d <= '1'; ram_we <= '1';
    wait until rising_edge(clk);
    wait for 1 ns;
    assert q_r = '1' and q_c = '1' report "FDRE and FDCE load" severity failure;
    assert spo = '1' and dpo = '1' report "RAM32X1D write" severity failure;
    rst <= '1';
    wait for 1 ns;
    assert q_c = '0' report "FDCE asynchronous clear" severity failure;
    assert q_r = '1' report "FDRE holds until the edge" severity failure;
    wait until rising_edge(clk);
    wait for 1 ns;
    assert q_r = '0' report "FDRE synchronous reset" severity failure;
    report "registers ok";

    i <= '1'; ib <= '0';
    wait for 1 ns;
    assert ibuf_o = '1' and obuf_o = '1' and o_ds = '1' and p = '1' and n = '0' report "buffers" severity failure;
    io_i <= '1';
    wait for 1 ns;
    assert io_pad = '1' and io_o = '1' report "IOBUF drives" severity failure;
    io_t <= '1';
    wait for 1 ns;
    assert io_pad = 'Z' report "IOBUF releases" severity failure;
    io_pad <= '0';
    wait for 1 ns;
    assert io_o = '0' report "IOBUF reads the pad" severity failure;
    io_pad <= 'Z';
    report "io buffers ok";

    lut_in <= "000000";
    wait for 1 ns;
    assert lut6 = '1' and lut5 = '1' report "LUT6_2 at 0" severity failure;
    lut_in <= "111111";
    wait for 1 ns;
    assert lut6 = '1' and lut5 = '0' report "LUT6_2 at 63" severity failure;
    assert mux7 = '0' report "MUXF7 selects I1" severity failure;
    assert sum = X"5" and co = X"F" report "CARRY4" severity failure;
    report "lut and carry ok";

    d <= '1'; rst <= '0';
    for k in 0 to 3 loop
      wait until rising_edge(clk);
    end loop;
    wait for 1 ns;
    assert srl_q = '1' and srl32_q = '1' report "shift registers" severity failure;
    report "shift registers ok";

    done <= true;
    report "done tb_v_basic";
    std.env.finish;
    wait;
  end process;
end architecture;
