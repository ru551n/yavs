-- The rest of the arithmetic macros, and the true dual port block RAM macro.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
library unisim;
use unisim.vcomponents.all;
library unimacro;
use unimacro.vcomponents.all;

entity tb_v_macro is
end entity;

architecture sim of tb_v_macro is
  signal clk : std_ulogic := '0';
  signal done : boolean := false;
  signal rst : std_ulogic := '1';
  signal a, b : std_logic_vector(47 downto 0) := (others => '0');
  signal sum : std_logic_vector(47 downto 0);
  signal count_tc, count_load : std_logic_vector(47 downto 0);
  signal tc, eq : std_logic;
  signal load : std_logic := '0';
  signal load_data : std_logic_vector(47 downto 0) := (others => '0');
  signal product : std_logic_vector(47 downto 0);
  signal pre1, pre2 : std_logic_vector(24 downto 0) := (others => '0');
  signal mul : std_logic_vector(17 downto 0) := (others => '0');
  signal we : std_logic_vector(1 downto 0) := "00";
  signal addr_a, addr_b : std_logic_vector(9 downto 0) := (others => '0');
  signal di_a : std_logic_vector(15 downto 0) := (others => '0');
  signal do_a, do_b : std_logic_vector(15 downto 0);
begin
  clk <= not clk after 5 ns when not done else '0';

  u_addsub : ADDSUB_MACRO
    generic map (DEVICE => "7SERIES", LATENCY => 2, WIDTH => 48)
    port map (RESULT => sum, A => a, ADD_SUB => '1', B => b, CARRYIN => '0', CE => '1', CLK => clk, RST => rst);
  u_tc : COUNTER_TC_MACRO
    generic map (DEVICE => "7SERIES", TC_VALUE => X"000000000005", WIDTH_DATA => 48)
    port map (Q => count_tc, TC => tc, CE => '1', CLK => clk, RST => rst);
  u_load : COUNTER_LOAD_MACRO
    generic map (DEVICE => "7SERIES", COUNT_BY => X"000000000002", WIDTH_DATA => 48)
    port map (Q => count_load, CE => '1', CLK => clk, DIRECTION => '1', LOAD => load, LOAD_DATA => load_data, RST => rst);
  u_addmacc : ADDMACC_MACRO
    generic map (DEVICE => "7SERIES", LATENCY => 4)
    port map (PRODUCT => product, CARRYIN => '0', CE => '1', CLK => clk, MULTIPLIER => mul, LOAD => load,
              LOAD_DATA => load_data, PREADD1 => pre1, PREADD2 => pre2, RST => rst);
  u_eq : EQ_COMPARE_MACRO
    generic map (DEVICE => "7SERIES", LATENCY => 2)
    port map (Q => eq, CE => '1', CLK => clk, DATA_IN => a, DYNAMIC_PATTERN => b, RST => rst);
  u_tdp : BRAM_TDP_MACRO
    generic map (BRAM_SIZE => "18Kb", DEVICE => "7SERIES", READ_WIDTH_A => 16, READ_WIDTH_B => 16, WRITE_WIDTH_A => 16,
                 WRITE_WIDTH_B => 16, SIM_COLLISION_CHECK => "NONE")
    port map (DOA => do_a, DOB => do_b, ADDRA => addr_a, ADDRB => addr_b, CLKA => clk, CLKB => clk, DIA => di_a,
              DIB => X"0000", ENA => '1', ENB => '1', REGCEA => '1', REGCEB => '1', RSTA => rst, RSTB => rst,
              WEA => we, WEB => "00");

  process
    procedure tick(n : positive := 1) is
    begin
      for k in 1 to n loop
        wait until rising_edge(clk);
      end loop;
      wait for 1 ns;
    end procedure;
  begin
    tick(3);
    rst <= '0';
    a <= X"000000001000";
    b <= X"000000000234";
    tick(4);
    report "sum " & to_hstring(sum) & " count_tc " & to_hstring(count_tc) & " tc " & to_string(tc)
      & " count_load " & to_hstring(count_load) & " eq " & to_string(eq);
    assert sum = X"000000001234" report "ADDSUB_MACRO" severity failure;
    assert eq = '0' report "EQ_COMPARE_MACRO differs" severity failure;
    b <= a;
    tick(4);
    assert eq = '1' report "EQ_COMPARE_MACRO equal" severity failure;
    assert unsigned(count_tc) > 5 or tc = '0' report "COUNTER_TC_MACRO" severity failure;

    load_data <= X"000000000100";
    load <= '1';
    tick;
    load <= '0';
    tick(3);
    report "count_load " & to_hstring(count_load);
    assert count_load = X"000000000104" report "COUNTER_LOAD_MACRO" severity failure;

    pre1 <= std_logic_vector(to_signed(100, 25));
    pre2 <= std_logic_vector(to_signed(23, 25));
    mul <= std_logic_vector(to_signed(2, 18));
    tick(6);
    report "product " & to_hstring(product);
    assert product = X"00000000031A" report "ADDMACC_MACRO" severity failure;

    addr_a <= 10x"005";
    addr_b <= 10x"005";
    di_a <= X"BEEF";
    we <= "11";
    tick;
    we <= "00";
    tick(2);
    assert do_a = X"BEEF" and do_b = X"BEEF" report "BRAM_TDP_MACRO" severity failure;

    done <= true;
    report "done tb_v_macro";
    std.env.finish;
    wait;
  end process;
end architecture;
