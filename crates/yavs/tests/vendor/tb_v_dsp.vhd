-- DSP48 slices and the arithmetic macros: multiply, multiply-add, pipeline depth.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
library unisim;
use unisim.vcomponents.all;
library unimacro;
use unimacro.vcomponents.all;

entity tb_v_dsp is
end entity;

architecture sim of tb_v_dsp is
  signal clk : std_ulogic := '0';
  signal done : boolean := false;
  signal rst : std_ulogic := '1';
  signal a : signed(24 downto 0) := (others => '0');
  signal b : signed(17 downto 0) := (others => '0');
  signal c : signed(47 downto 0) := (others => '0');
  signal p_e1, p_e2, p_macc : std_logic_vector(47 downto 0);
  signal p_mult : std_logic_vector(42 downto 0);
  signal a30 : std_logic_vector(29 downto 0);
  signal a_macc : std_logic_vector(24 downto 0);
  signal b_slv : std_logic_vector(17 downto 0);
  signal c_slv : std_logic_vector(47 downto 0);
  signal opmode_e1 : std_logic_vector(6 downto 0) := "0110101";
  signal opmode_e2 : std_logic_vector(8 downto 0) := "000110101";
  signal sum : std_logic_vector(47 downto 0);
begin
  clk <= not clk after 5 ns when not done else '0';
  a30 <= std_logic_vector(resize(a, 30));
  a_macc <= std_logic_vector(a);
  b_slv <= std_logic_vector(b);
  c_slv <= std_logic_vector(c);

  -- P = A * B + C with AREG = BREG = CREG = 1, MREG = PREG = 1: three clocks.
  u_e1 : DSP48E1
    port map (P => p_e1, A => a30, ACIN => (others => '0'), ALUMODE => "0000", B => b_slv, BCIN => (others => '0'),
              C => c_slv, CARRYCASCIN => '0', CARRYIN => '0', CARRYINSEL => "000", CEA1 => '0', CEA2 => '1', CEAD => '0',
              CEALUMODE => '1', CEB1 => '0', CEB2 => '1', CEC => '1', CECARRYIN => '1', CECTRL => '1', CED => '0',
              CEINMODE => '1', CEM => '1', CEP => '1', CLK => clk, D => (others => '0'), INMODE => "00000",
              MULTSIGNIN => '0', OPMODE => opmode_e1, PCIN => (others => '0'), RSTA => rst, RSTALLCARRYIN => rst,
              RSTALUMODE => rst, RSTB => rst, RSTC => rst, RSTCTRL => rst, RSTD => rst, RSTINMODE => rst, RSTM => rst, RSTP => rst);
  u_e2 : DSP48E2
    port map (P => p_e2, A => a30, ACIN => (others => '0'), ALUMODE => "0000", B => b_slv, BCIN => (others => '0'),
              C => c_slv, CARRYCASCIN => '0', CARRYIN => '0', CARRYINSEL => "000", CEA1 => '0', CEA2 => '1', CEAD => '0',
              CEALUMODE => '1', CEB1 => '0', CEB2 => '1', CEC => '1', CECARRYIN => '1', CECTRL => '1', CED => '0',
              CEINMODE => '1', CEM => '1', CEP => '1', CLK => clk, D => (others => '0'), INMODE => "00000",
              MULTSIGNIN => '0', OPMODE => opmode_e2, PCIN => (others => '0'), RSTA => rst, RSTALLCARRYIN => rst,
              RSTALUMODE => rst, RSTB => rst, RSTC => rst, RSTCTRL => rst, RSTD => rst, RSTINMODE => rst, RSTM => rst, RSTP => rst);
  u_macc : MACC_MACRO
    generic map (DEVICE => "7SERIES", LATENCY => 3, WIDTH_A => 25, WIDTH_B => 18, WIDTH_P => 48)
    port map (P => p_macc, A => a_macc, ADDSUB => '1', B => b_slv, CARRYIN => '0', CE => '1', CLK => clk,
              LOAD => rst, LOAD_DATA => (others => '0'), RST => rst);
  u_mult : MULT_MACRO
    generic map (DEVICE => "7SERIES", LATENCY => 3, WIDTH_A => 25, WIDTH_B => 18)
    port map (P => p_mult, A => a_macc, B => b_slv, CE => '1', CLK => clk, RST => rst);

  process
    procedure tick(n : positive := 1) is
    begin
      for k in 1 to n loop
        wait until rising_edge(clk);
      end loop;
      wait for 1 ns;
    end procedure;
    variable expect : signed(47 downto 0);
  begin
    tick(3);
    rst <= '0';
    tick;
    for k in -3 to 3 loop
      a <= to_signed(1000 * k + 7, 25);
      b <= to_signed(-300 * k + 11, 18);
      c <= to_signed(1000000 * k, 48);
      expect := resize(to_signed(1000 * k + 7, 25) * to_signed(-300 * k + 11, 18), 48) + to_signed(1000000 * k, 48);
      tick(4);
      report "k " & integer'image(k) & " e1 " & to_hstring(p_e1) & " e2 " & to_hstring(p_e2) & " mult " & to_hstring(p_mult);
      assert signed(p_e1) = expect report "DSP48E1" severity failure;
      assert signed(p_e2) = expect report "DSP48E2" severity failure;
      assert signed(p_mult) = resize(to_signed(1000 * k + 7, 25) * to_signed(-300 * k + 11, 18), 43) report "MULT_MACRO" severity failure;
    end loop;
    -- The accumulator's running sum of the products (sampled every fourth clock above).
    report "macc " & to_hstring(p_macc);
    assert p_macc = X"FFFFFE50EAFA" report "MACC_MACRO" severity failure;
    done <= true;
    report "done tb_v_dsp";
    std.env.finish;
    wait;
  end process;
end architecture;
