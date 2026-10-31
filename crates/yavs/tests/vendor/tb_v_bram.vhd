-- Block RAM: initial contents, write and read on both ports, output register, macro wrappers.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
library unisim;
use unisim.vcomponents.all;
library unimacro;
use unimacro.vcomponents.all;

entity tb_v_bram is
end entity;

architecture sim of tb_v_bram is
  signal clk : std_ulogic := '0';
  signal done : boolean := false;
  signal rst : std_ulogic := '1';
  signal we : std_logic_vector(3 downto 0) := "0000";
  signal addr : unsigned(8 downto 0) := (others => '0');
  signal din : std_logic_vector(31 downto 0) := (others => '0');
  signal q36e1, q36e2, q36e2_b : std_logic_vector(31 downto 0);
  signal q18e1 : std_logic_vector(15 downto 0);
  signal q18e2 : std_logic_vector(15 downto 0);
  signal q_sdp : std_logic_vector(31 downto 0);
  signal q_single : std_logic_vector(15 downto 0);
  signal a36 : std_logic_vector(15 downto 0);
  signal a36e2 : std_logic_vector(14 downto 0);
  signal a18 : std_logic_vector(13 downto 0);
  signal a_sdp : std_logic_vector(9 downto 0);
  signal a_single : std_logic_vector(9 downto 0);
begin
  clk <= not clk after 5 ns when not done else '0';
  a36 <= "00" & std_logic_vector(addr) & "00000";
  a36e2 <= "0" & std_logic_vector(addr) & "00000";
  a18 <= std_logic_vector(addr) & "00000";
  a_sdp <= "0" & std_logic_vector(addr);
  a_single <= std_logic_vector(addr) & "0";

  u_36e1 : RAMB36E1
    generic map (READ_WIDTH_A => 36, WRITE_WIDTH_A => 36, DOA_REG => 0, INIT_00 => X"00000000000000000000000000000000000000000000000000000000DEADBEEF",
                 SIM_DEVICE => "7SERIES")
    port map (DOADO => q36e1, ADDRARDADDR => a36, ADDRBWRADDR => (others => '0'), CASCADEINA => '0', CASCADEINB => '0',
              CLKARDCLK => clk, CLKBWRCLK => '0', DIADI => din, DIBDI => (others => '0'), DIPADIP => "0000",
              DIPBDIP => "0000", ENARDEN => '1', ENBWREN => '0', INJECTDBITERR => '0', INJECTSBITERR => '0',
              REGCEAREGCE => '1', REGCEB => '0', RSTRAMARSTRAM => rst, RSTRAMB => '0', RSTREGARSTREG => '0',
              RSTREGB => '0', WEA => we, WEBWE => (others => '0'));
  u_18e1 : RAMB18E1
    generic map (READ_WIDTH_A => 18, WRITE_WIDTH_A => 18, DOA_REG => 0, INIT_00 => X"000000000000000000000000000000000000000000000000000000000000CAFE",
                 SIM_DEVICE => "7SERIES")
    port map (DOADO => q18e1, ADDRARDADDR => a18, ADDRBWRADDR => (others => '0'), CLKARDCLK => clk, CLKBWRCLK => '0',
              DIADI => din(15 downto 0), DIBDI => (others => '0'), DIPADIP => "00", DIPBDIP => "00", ENARDEN => '1',
              ENBWREN => '0', REGCEAREGCE => '1', REGCEB => '0', RSTRAMARSTRAM => rst, RSTRAMB => '0',
              RSTREGARSTREG => '0', RSTREGB => '0', WEA => we(1 downto 0), WEBWE => "0000");
  u_36e2 : RAMB36E2
    generic map (READ_WIDTH_A => 36, WRITE_WIDTH_A => 36, READ_WIDTH_B => 36, DOA_REG => 1, DOB_REG => 0, SIM_COLLISION_CHECK => "NONE",
                 INIT_00 => X"00000000000000000000000000000000000000000000000000000000FEEDF00D")
    port map (DOUTADOUT => q36e2, DOUTBDOUT => q36e2_b, ADDRARDADDR => a36e2, ADDRBWRADDR => a36e2, ADDRENA => '1', ADDRENB => '1',
              CASDIMUXA => '0', CASDIMUXB => '0', CASDINA => (others => '0'), CASDINB => (others => '0'),
              CASDINPA => "0000", CASDINPB => "0000", CASDOMUXA => '0', CASDOMUXB => '0', CASDOMUXEN_A => '1',
              CASDOMUXEN_B => '1', CASINDBITERR => '0', CASINSBITERR => '0', CASOREGIMUXA => '0', CASOREGIMUXB => '0',
              CASOREGIMUXEN_A => '1', CASOREGIMUXEN_B => '1', CLKARDCLK => clk, CLKBWRCLK => clk, DINADIN => din,
              DINBDIN => (others => '0'), DINPADINP => "0000", DINPBDINP => "0000", ECCPIPECE => '0', ENARDEN => '1',
              ENBWREN => '1', INJECTDBITERR => '0', INJECTSBITERR => '0', REGCEAREGCE => '1', REGCEB => '0',
              RSTRAMARSTRAM => rst, RSTRAMB => rst, RSTREGARSTREG => rst, RSTREGB => '0', SLEEP => '0', WEA => we,
              WEBWE => (others => '0'));
  u_18e2 : RAMB18E2
    generic map (READ_WIDTH_A => 18, WRITE_WIDTH_A => 18, DOA_REG => 0, INIT_00 => X"000000000000000000000000000000000000000000000000000000000000BABE")
    port map (DOUTADOUT => q18e2, ADDRARDADDR => a18, ADDRBWRADDR => (others => '0'), ADDRENA => '1', ADDRENB => '1',
              CASDIMUXA => '0', CASDIMUXB => '0', CASDINA => (others => '0'), CASDINB => (others => '0'),
              CASDINPA => "00", CASDINPB => "00", CASDOMUXA => '0', CASDOMUXB => '0', CASDOMUXEN_A => '1',
              CASDOMUXEN_B => '1', CASOREGIMUXA => '0', CASOREGIMUXB => '0', CASOREGIMUXEN_A => '1', CASOREGIMUXEN_B => '1',
              CLKARDCLK => clk, CLKBWRCLK => '0', DINADIN => din(15 downto 0), DINBDIN => (others => '0'),
              DINPADINP => "00", DINPBDINP => "00", ENARDEN => '1', ENBWREN => '0', REGCEAREGCE => '1', REGCEB => '0',
              RSTRAMARSTRAM => rst, RSTRAMB => '0', RSTREGARSTREG => '0', RSTREGB => '0', SLEEP => '0',
              WEA => we(1 downto 0), WEBWE => "0000");
  u_sdp : BRAM_SDP_MACRO
    generic map (BRAM_SIZE => "36Kb", DEVICE => "7SERIES", WRITE_WIDTH => 32, READ_WIDTH => 32, DO_REG => 0, SIM_COLLISION_CHECK => "NONE")
    port map (DO => q_sdp, DI => din, RDADDR => a_sdp, RDCLK => clk, RDEN => '1', REGCE => '1',
              RST => rst, WE => we, WRADDR => a_sdp, WRCLK => clk, WREN => we(0));
  u_single : BRAM_SINGLE_MACRO
    generic map (BRAM_SIZE => "18Kb", DEVICE => "7SERIES", WRITE_WIDTH => 16, READ_WIDTH => 16, DO_REG => 0)
    port map (DO => q_single, ADDR => a_single, CLK => clk, DI => din(15 downto 0),
              EN => '1', REGCE => '1', RST => rst, WE => we(1 downto 0));

  process
    procedure tick is
    begin
      wait until rising_edge(clk);
      wait for 1 ns;
    end procedure;
  begin
    tick; tick;
    rst <= '0';
    tick; tick; tick;
    report "init 36e1 " & to_hstring(q36e1) & " 18e1 " & to_hstring(q18e1)
      & " 36e2 " & to_hstring(q36e2) & " 18e2 " & to_hstring(q18e2);
    assert q36e1 = X"DEADBEEF" and q18e1 = X"CAFE" and q18e2 = X"BABE" and q36e2 = X"FEEDF00D"
      report "INIT_00" severity failure;

    -- Write a pattern, read it back: the two ports of the 36E2 see the same contents.
    for k in 1 to 16 loop
      addr <= to_unsigned(k, addr'length);
      din <= std_logic_vector(to_unsigned(k * 16#01010101#, 32));
      we <= "1111";
      tick;
    end loop;
    we <= "0000";
    for k in 1 to 16 loop
      addr <= to_unsigned(k, addr'length);
      tick; tick;
      if k = 3 or k = 16 then
        report "at " & integer'image(k) & ": " & to_hstring(q36e1) & " " & to_hstring(q18e1) & " " & to_hstring(q36e2)
          & " " & to_hstring(q36e2_b) & " " & to_hstring(q18e2) & " " & to_hstring(q_sdp) & " " & to_hstring(q_single);
      end if;
      assert q36e1 = std_logic_vector(to_unsigned(k * 16#01010101#, 32)) report "36E1 read" severity failure;
      assert q36e2 = q36e1 report "36E2 port A" severity failure;
      assert q36e2_b = q36e1 report "36E2 port B" severity failure;
      assert q18e1 = q36e1(15 downto 0) report "18E1 read" severity failure;
      assert q18e2 = q36e1(15 downto 0) report "18E2 read" severity failure;
      assert q_sdp = q36e1 report "SDP macro read" severity failure;
      assert q_single = q36e1(15 downto 0) report "single macro read" severity failure;
    end loop;

    -- Byte write enable.
    addr <= to_unsigned(1, addr'length);
    din <= X"AABBCCDD";
    we <= "0010";
    tick;
    we <= "0000";
    tick; tick;
    report "byte write " & to_hstring(q36e1);
    assert q36e1 = X"0101CC01" report "byte write enable" severity failure;

    done <= true;
    report "done tb_v_bram";
    std.env.finish;
    wait;
  end process;
end architecture;
