-- Memories (signals of more than 4096 logic elements, or more than 64 integers) written by clocked
-- members at run-time indices, in cycle mode against the normal mode (`W` is replaced by the width
-- of a word): a read of the address written at the same edge gives the old value, in the writing
-- process, in another process and on a 2x clock from the same clock process; two writes of one
-- edge (the later one wins at the same address), a conditional write, a write of part of a word,
-- a combinational read. A memory the testbench reads stays out.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_ram is
end entity;
architecture a of cycle_ram is
  constant depth : positive := 4096 / W + 2;
  type mem_t is array (0 to depth - 1) of std_ulogic_vector(W - 1 downto 0);
  signal clk, clk2 : std_ulogic := '0';
  signal we : std_ulogic := '0';
  signal wa, wb, ra : natural range 0 to depth - 1 := 0;
  signal d1, d2 : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal nib : std_ulogic_vector(0 downto 0) := "0";
  signal mem : mem_t := (others => (others => '0'));
  signal seen : mem_t := (others => (others => '0'));
  signal q1, q2, q3, qc, qs : std_ulogic_vector(W - 1 downto 0);
begin
  writer : process (clk)
  begin
    if rising_edge(clk) then
      if we = '1' then
        mem(wa) <= d1;
      end if;
      mem(wb) <= d2;
      mem((wa + 1) mod depth)(0 downto 0) <= nib;
      q1 <= mem(wa);
    end if;
  end process;

  reader : process (clk)
  begin
    if rising_edge(clk) then
      q2 <= mem(wb);
    end if;
  end process;

  fast : process (clk2)
  begin
    if rising_edge(clk2) then
      q3 <= mem(ra);
    end if;
  end process;

  qc <= mem(ra);

  -- The testbench reads `seen` itself: its writer stays out.
  shadow : process (clk)
  begin
    if rising_edge(clk) then
      seen(wa) <= d1;
      qs <= seen(ra);
    end if;
  end process;

  clocks : process
  begin
    for c in 1 to 80 loop
      clk2 <= not clk2;
      if c mod 2 = 1 then
        clk <= not clk;
      end if;
      wait for 5 ns;
    end loop;
    wait;
  end process;

  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for c in 1 to 20 loop
      wait until rising_edge(clk);
      wait for 2 ns;
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      we <= r(0);
      wa <= to_integer(r(11 downto 4)) mod 8;
      wb <= (to_integer(r(15 downto 8)) mod 4) * 2;
      ra <= to_integer(r(9 downto 2)) mod 8;
      d1 <= std_ulogic_vector(resize(r, W) rol c);
      d2 <= std_ulogic_vector(resize(not r, W));
      nib(0) <= r(3);
      report "c=" & integer'image(c) & " q1=" & to_hstring(q1) & " q2=" & to_hstring(q2)
        & " q3=" & to_hstring(q3) & " qc=" & to_hstring(qc) & " qs=" & to_hstring(qs)
        & " s=" & to_hstring(seen(3));
    end loop;
    wait;
  end process;
end architecture;
