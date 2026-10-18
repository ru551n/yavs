-- difftest: top=sig_attrs std=2008
library ieee;
use ieee.std_logic_1164.all;
entity sig_attrs is end;
architecture t of sig_attrs is
  signal a : std_ulogic_vector(3 downto 0) := "0000";
  signal k : integer := 0;
begin
  process
  begin
    a(2) <= '1';
    k <= 3;
    wait for 0 ns;
    report "ev=" & boolean'image(a'event) & boolean'image(a(2)'event) & boolean'image(a(1)'event)
      & " last=" & to_string(a'last_value) & " " & std_ulogic'image(a(2)'last_value)
      & " k=" & integer'image(k'last_value) & " slice=" & boolean'image(a(3 downto 2)'active);
    wait for 2 ns;
    report "le=" & time'image(a(2)'last_event) & " " & boolean'image(a'event);
    wait;
  end process;
end;
