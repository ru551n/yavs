-- difftest: top=tb_logic_gates std=2008
-- Concurrent assignments of one logical operator on signal names (#779): values over all nine
-- std_ulogic values, deltas, events, a resolved target with two such drivers, an in port, vectors
-- wider than 64 elements, BIT and BOOLEAN, and inertial and transport delays with pulses shorter
-- than the delay must be those of the equivalent processes.
library ieee;
use ieee.std_logic_1164.all;

entity gate_inner is
  port (i : in std_ulogic; o : out std_ulogic);
end;

architecture a of gate_inner is
begin
  o <= not i;
end;

library ieee;
use ieee.std_logic_1164.all;

entity tb_logic_gates is end;

architecture t of tb_logic_gates is
  signal a, b : std_logic := '0';
  signal y_not, y_and, y_or, y_xor, y_nand, y_nor, y_xnor, y_in, r : std_logic;
  signal y_in2, y_tr : std_logic;
  signal v1 : std_logic_vector(3 downto 0) := "0000";
  signal v2 : std_logic_vector(0 to 3) := "0000";
  signal v3 : std_logic_vector(3 downto 0);
  signal v4 : std_ulogic_vector(1 downto 0);
  signal w1, w2, w3 : std_logic_vector(69 downto 0) := (others => '0');
  signal ba, bc, bb : bit;
  signal bl, bo : boolean;
  signal bv1, bv2, bv3 : bit_vector(2 downto 0);
  signal p, p_in, p_tr : std_logic := '0';
  signal step : natural := 0;
begin
  u : entity work.gate_inner port map (i => a, o => y_in);
  y_not <= not a;
  y_and <= a and b;
  y_or <= a or b;
  y_xor <= a xor b;
  y_nand <= a nand b;
  y_nor <= a nor b;
  y_xnor <= a xnor b;
  r <= a and b;
  r <= a or b;
  v3 <= v1 or v2;
  v4 <= v1(3 downto 2) nor v2(0 to 1);
  w3 <= w1 xnor w2;
  bb <= ba xor bc;
  bo <= not bl;
  bv3 <= bv1 nand bv2;
  p_in <= not p after 3 ns;
  p_tr <= transport not p after 3 ns;

  stim : process
  begin
    for i in 0 to 80 loop
      a <= std_logic'val(i / 9);
      b <= std_logic'val(i mod 9);
      v1 <= (std_logic'val(i mod 9), std_logic'val(i / 9), std_logic'val((i + 4) mod 9), '1');
      v2 <= (std_logic'val((i * 7) mod 9), 'H', std_logic'val(i / 9), std_logic'val(i mod 9));
      w1 <= (others => std_logic'val(i mod 9));
      w1(64) <= std_logic'val(i / 9);
      w2 <= (others => std_logic'val(i / 9));
      ba <= bit'val(i mod 2);
      bc <= bit'val(i / 2 mod 2);
      bl <= i mod 3 = 0;
      bv1 <= (bit'val(i mod 2), bit'val(i / 2 mod 2), '1');
      bv2 <= (bit'val(i / 4 mod 2), '1', bit'val(i mod 2));
      step <= i;
      wait for 1 ns;
    end loop;
    -- Pulses of 1, 2 and 4 ns into gates with a 3 ns delay.
    for k in 1 to 3 loop
      p <= '1';
      wait for k * 1 ns + (k / 3) * 1 ns;
      p <= '0';
      wait for 5 ns;
    end loop;
    assert y_not = 'X' and y_in = 'X' and r = 'X' and v4 = "X0" severity failure;
    report "done";
    wait;
  end process;

  watch : process (y_not, y_and, y_or, y_xor, y_nand, y_nor, y_xnor, y_in, r, v3, v4, w3, bb, bo, bv3)
  begin
    report integer'image(step) & " " & to_string(y_not) & to_string(y_and) & to_string(y_or)
      & to_string(y_xor) & to_string(y_nand) & to_string(y_nor) & to_string(y_xnor)
      & to_string(y_in) & to_string(r) & " " & to_string(v3) & " " & to_string(v4) & " "
      & to_string(w3(69 downto 62)) & to_string(w3(1 downto 0)) & " " & to_string(bb)
      & boolean'image(bo) & " " & to_string(bv3) & " ev=" & boolean'image(y_and'event)
      & boolean'image(r'event) & boolean'image(w3'event);
  end process;

  pulses : process (p_in, p_tr)
  begin
    report "p_in=" & to_string(p_in) & " p_tr=" & to_string(p_tr);
  end process;

  check : process (a, b)
  begin
    -- 'H' and '1' is '1'; 'U' wins over 'X' in and/or, '0' wins over 'U' in and.
    if a = 'H' and b = '1' then
      assert (a and b) = '1' severity failure;
    end if;
    if a = '0' and b = 'U' then
      assert (a and b) = '0' severity failure;
    end if;
  end process;
end;
