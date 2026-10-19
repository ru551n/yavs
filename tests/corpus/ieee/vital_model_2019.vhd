-- difftest: top=vital_model_2019 std=2019
-- ieee.vital_timing / vital_primitives (IEEE 1076.4) in a zero-delay VITAL-style model:
-- a NAND gate and a mux built from primitives, a flip-flop whose output goes through
-- VitalPathDelay01 with zero path delays, and VitalSetupHoldCheck on the data input.
library ieee; use ieee.std_logic_1164.all; use ieee.vital_timing.all; use ieee.vital_primitives.all;
entity vital_dff is
  generic (tpd_clk_q : VitalDelayType01 := (0 ns, 0 ns);
           tsetup_d_clk : VitalDelayType := 0 ns;
           thold_d_clk : VitalDelayType := 0 ns;
           TimingChecksOn : boolean := true);
  port (d, clk : in std_logic; q : out std_logic);
  attribute VITAL_LEVEL0 of vital_dff : entity is true;
end;
architecture vital of vital_dff is
  attribute VITAL_LEVEL1 of vital : architecture is true;
  signal d_ipd, clk_ipd : std_ulogic := 'U';
begin
  d_ipd <= d; clk_ipd <= clk;
  process (d_ipd, clk_ipd)
    variable tviol : X01 := '0';
    variable tdata : VitalTimingDataType;
    variable q_gd : std_logic := 'U';
    variable q_gl : VitalGlitchDataType;
    variable q_zd : std_logic := 'U';
  begin
    if TimingChecksOn then
      VitalSetupHoldCheck(Violation => tviol, TimingData => tdata, TestSignal => d_ipd,
        TestSignalName => "d", RefSignal => clk_ipd, RefSignalName => "clk",
        SetupHigh => tsetup_d_clk, SetupLow => tsetup_d_clk, HoldHigh => thold_d_clk,
        HoldLow => thold_d_clk, CheckEnabled => true, RefTransition => '/',
        HeaderMsg => "vital_dff", XOn => true, MsgOn => true, MsgSeverity => warning);
    end if;
    if rising_edge(clk_ipd) then q_zd := to_x01(d_ipd); end if;
    VitalPathDelay01(OutSignal => q, OutSignalName => "q", OutTemp => q_zd,
      Paths => (0 => (InputChangeTime => clk_ipd'last_event, PathDelay => tpd_clk_q,
                      PathCondition => true)),
      GlitchData => q_gl, Mode => OnEvent, Xon => true, MsgOn => true);
  end process;
end;

library ieee; use ieee.std_logic_1164.all; use ieee.vital_primitives.all;
entity vital_model_2019 is end;
architecture t of vital_model_2019 is
  signal a, b, s, n, m, q, clk : std_logic := '0';
  signal d : std_logic;
begin
  n <= VitalNAND2(a, b);
  m <= VitalMUX2(data0 => a, data1 => b, dselect => s);
  d <= n xor m;
  u : entity work.vital_dff port map (d => d, clk => clk, q => q);
  clk <= not clk after 5 ns when now < 80 ns else clk;
  process
  begin
    for i in 0 to 7 loop
      s <= '1' when (i / 4) mod 2 = 1 else '0';
      a <= '1' when (i / 2) mod 2 = 1 else '0';
      b <= '1' when i mod 2 = 1 else '0';
      wait for 8 ns;
      report "i " & integer'image(i) & " n " & std_logic'image(n) & " m " & std_logic'image(m)
             & " d " & std_logic'image(d) & " q " & std_logic'image(q);
    end loop;
    wait;
  end process;
end;
