-- Slices of a run-time length returned by small functions, as cache index helpers do: the bounds
-- of the slice from generics and from the arguments. Compiled code copies the elements in place
-- on the function's native frame, with no call of the runtime's copy.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity clift_slice_len is
  generic (g_hi : integer := 13; g_lo : integer := 3);
end;
architecture a of clift_slice_len is
  subtype row_t is unsigned(g_hi - 1 - g_lo downto 0);
  function get_row(addr : std_ulogic_vector) return row_t is
  begin
    return unsigned(addr(g_hi - 1 downto g_lo));
  end;
  function field(v : std_ulogic_vector; hi, lo : natural) return unsigned is
  begin
    return unsigned(v(hi downto lo));
  end;
begin
  process is
    variable a : std_ulogic_vector(63 downto 0) := x"0123456789abcdef";
    variable s : natural := 0;
  begin
    for i in 0 to 199 loop
      a := std_ulogic_vector(unsigned(a) + 40503);
      s := (s + to_integer(get_row(a)) + to_integer(field(a, 20 + i mod 7, 17))) mod 1000003;
    end loop;
    report "rows=" & integer'image(s);
    wait;
  end process;
end;
