-- difftest: top=port_part_formal_conv
-- A conversion of a part of a formal in a port map (6.5.7.1, #617): type marks on slices of an
-- `out` port, functions on its elements, both conversions on slices of an `inout` port.
library ieee; use ieee.std_logic_1164.all;
entity ppfc_out is port (res : out std_logic_vector(15 downto 0); o : out std_logic_vector(1 downto 0)); end entity;
architecture a of ppfc_out is begin
  res <= x"1234";
  o <= "10";
end architecture;

library ieee; use ieee.std_logic_1164.all;
entity ppfc_io is port (io : inout std_logic_vector(7 downto 0)); end entity;
architecture a of ppfc_io is begin
  io <= x"a5";
  process begin
    wait for 1 ns;
    report to_string(io);
    wait;
  end process;
end architecture;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity port_part_formal_conv is end entity;
architecture a of port_part_formal_conv is
  signal res_a, res_b : signed(7 downto 0);
  signal b0, b1 : bit;
  signal x, y : signed(3 downto 0);
  function tobit(s : std_ulogic) return bit is begin return to_bit(s); end function;
begin
  u : entity work.ppfc_out port map (
    signed(res(7 downto 0)) => res_a,
    signed(res(15 downto 8)) => res_b,
    tobit(o(0)) => b0,
    tobit(o(1)) => b1);
  v : entity work.ppfc_io port map (
    signed(io(3 downto 0)) => std_logic_vector(x),
    signed(io(7 downto 4)) => std_logic_vector(y));
  process begin
    wait for 2 ns;
    report integer'image(to_integer(res_a)) & " " & integer'image(to_integer(res_b));
    report bit'image(b0) & " " & bit'image(b1);
    report to_string(x) & " " & to_string(y);
    wait;
  end process;
end architecture;
