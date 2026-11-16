-- A memory of more than 2**20 elements (1025 words of 1025 bits) written and read at run-time
-- indices, as a first-word-fall-through FIFO's RAM is: it joins the region, its writes kept back
-- for the commit, never copied whole per edge.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_bigram is
end entity;
architecture a of cycle_bigram is
  subtype word_t is std_ulogic_vector(1024 downto 0);
  type mem_t is array (0 to 1024) of word_t;
  signal mem : mem_t := (others => (others => '0'));
  signal clk : std_ulogic := '0';
  signal wa, ra : natural range 0 to 1024 := 0;
  signal wd, rd : word_t := (others => '0');
begin
  ram : process (clk)
  begin
    if rising_edge(clk) then
      mem(wa) <= wd;
      rd <= mem(ra);
    end if;
  end process;
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for c in 1 to 40 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      wa <= (c * 97) mod 1025;
      ra <= ((c - 3) * 97) mod 1025;
      wd <= wd(1023 downto 0) & r(0);
      wd(1024) <= r(7);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "c=" & integer'image(c) & " rd=" & to_hstring(rd(1024 downto 960)) & " "
        & to_hstring(rd(63 downto 0));
    end loop;
    wait;
  end process;
end architecture;
