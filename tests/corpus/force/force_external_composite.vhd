-- difftest: top=force_external_composite std=2008
-- Force and release through aliases of external names of unconstrained array signals, the
-- form a verification library uses to inject errors on interface signals (#68).
library ieee; use ieee.std_logic_1164.all;
entity fec_dut is end entity;
architecture a of fec_dut is
  signal data : std_logic_vector(7 downto 0) := x"00";
  signal valid : std_logic := '0';
begin
  data <= x"3c" after 1 ns;
  valid <= '1' after 1 ns;
end architecture;
library ieee; use ieee.std_logic_1164.all;
entity force_external_composite is end entity;
architecture a of force_external_composite is
begin
  dut : entity work.fec_dut;
  process
    alias d is << signal dut.data : std_logic_vector >>;
    alias v is << signal dut.valid : std_logic >>;
  begin
    wait for 2 ns;
    d <= force not d;
    v <= force not v;
    wait for 1 ns;
    report "d=" & to_hstring(d) & " v=" & std_logic'image(v);
    assert d = x"c3" and v = '0' severity failure;
    d <= release;
    v <= release;
    wait for 1 ns;
    report "d=" & to_hstring(d) & " v=" & std_logic'image(v);
    assert d = x"3c" and v = '1' severity failure;
    wait;
  end process;
end architecture;
