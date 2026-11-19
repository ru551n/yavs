-- Parts of elements of a memory chosen at run time (`W` replaced by a width): the low byte and all
-- but the top bit of an element read, and the low byte of another element written, at run-time
-- indices, in a register and in combinational reads, extended and compared (the bits above a part
-- must be zero); elements start on words at widths 64 and 128.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_elempart is
end entity;
architecture t of cycle_elempart is
  type mem_t is array (0 to 7) of std_ulogic_vector(W - 1 downto 0);
  signal mem : mem_t := (others => (others => '0'));
  signal clk : std_ulogic := '0';
  signal wa, ra : natural range 0 to 7 := 0;
  signal d : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal lo8, lo8c : std_ulogic_vector(7 downto 0);
  signal most : std_ulogic_vector(W - 2 downto 0);
  signal wide : std_ulogic_vector(15 downto 0);
  signal z, zc : std_ulogic;
begin
  regs : process (clk)
  begin
    if rising_edge(clk) then
      mem(wa) <= d;
      mem((wa + 3) mod 8)(7 downto 0) <= d(15 downto 8);
      lo8 <= mem(ra)(7 downto 0);
      most <= mem(ra)(W - 2 downto 0);
      -- The bits above a part must be zero: an extension and a compare see them.
      wide <= std_ulogic_vector(resize(unsigned(mem(ra)(7 downto 0)), 16));
      z <= '1' when mem(ra)(W - 2 downto 0) = (W - 2 downto 0 => '0') else '0';
    end if;
  end process;
  lo8c <= mem(ra)(7 downto 0);
  zc <= '1' when mem(ra)(7 downto 0) = x"00" else '0';
  main : process
    variable s : unsigned(W - 1 downto 0) := (others => '1');
  begin
    for k in 1 to 12 loop
      s := s(W - 2 downto 0) & (s(W - 1) xor s(W / 2) xor s(1) xor s(0));
      d <= std_ulogic_vector(s);
      wa <= (k * 5) mod 8;
      ra <= (k * 3) mod 8;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " lo8=" & to_hstring(lo8) & " lo8c=" & to_hstring(lo8c)
        & " most=" & to_hstring(most) & " wide=" & to_hstring(wide) & " z=" & std_ulogic'image(z)
        & " zc=" & std_ulogic'image(zc);
    end loop;
    wait;
  end process;
end architecture;
