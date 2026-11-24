-- Combinational choices on a 'U' without --two-state never show a 'U' the normal mode does
-- not have. `s` and `ad` are 'U' (never driven), `uu` too: the normal mode takes the else branch
-- of `s = '0'`, `s = '1'`, `s /= '1'` and `unsigned(ad) < 2` and gives "0..0", while the region's
-- image reads 'U' as 0 and takes the `uu` branch of the first and the last. `yd` takes `uu`
-- without a choice and shows 'U' as the normal mode does.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity cycle_railcomb is
  generic (W : positive := 8);
end;
architecture a of cycle_railcomb is
  signal s : std_ulogic;
  signal ad : std_ulogic_vector(1 downto 0);
  signal uu : std_ulogic_vector(W - 1 downto 0);
  signal zero : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal y0, y1, yn, yl, yd : std_ulogic_vector(W - 1 downto 0);
begin
  y0 <= uu when s = '0' else zero;
  y1 <= uu when s = '1' else zero;
  yn <= zero when s /= '1' else uu;
  yl <= uu when unsigned(ad) < 2 else zero;
  yd <= uu or zero;
  stim : process begin
    wait for 1 ns;
    report "y0=" & to_string(y0) & " y1=" & to_string(y1) & " yn=" & to_string(yn)
      & " yl=" & to_string(yl) & " yd=" & to_string(yd);
    wait;
  end process;
end;
