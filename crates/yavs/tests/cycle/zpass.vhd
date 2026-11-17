-- A 'Z' of a tri-state line that a process outside copies (with a delay, which keeps it out of
-- the region) reaches a register the testbench reads: that register stays in the kernel, so the
-- testbench sees the 'Z' the normal mode gives; a register of a line without one joins, and so
-- does one of a tri-state line with a pull-up, which never resolves to 'Z', and one of a copy
-- outside that only tests the line. Copies through a variable and through `to_x01`, and a line
-- pulled up on one element only, keep their readers out. `W` is replaced by a width.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_zpass is
end entity;
architecture a of cycle_zpass is
  signal clk, en : std_ulogic := '0';
  signal pad, pull : std_logic_vector(W - 1 downto 0);
  signal mid, clean, q, r, p : std_ulogic_vector(W - 1 downto 0);
  signal tested, c : std_ulogic;
  signal vmid, xmid, qv, qx, qh : std_ulogic_vector(W - 1 downto 0);
  signal half : std_logic_vector(1 downto 0);
  signal k : unsigned(W - 1 downto 0) := (others => '0');
begin
  pad <= std_logic_vector(k) when en = '1' else (others => 'Z');
  mid <= pad after 1 ns;
  clean <= std_ulogic_vector(k + 3) after 1 ns;
  pull <= std_logic_vector(k) when en = '1' else (others => 'Z');
  pull <= (others => 'H');
  tested <= '0' when pad(0) = '0' else '1' after 1 ns;
  xmid <= to_x01(pad) after 1 ns;
  half <= pad(0) & pad(0) when en = '1' else "ZZ";
  half(0) <= 'H';

  copy_v : process (pad)
    variable v : std_ulogic_vector(W - 1 downto 0);
  begin
    v := pad;
    vmid <= v after 1 ns;
  end process;

  reg_q : process (clk)
  begin
    if rising_edge(clk) then
      q <= mid;
    end if;
  end process;

  reg_r : process (clk)
  begin
    if rising_edge(clk) then
      r <= not clean;
    end if;
  end process;

  reg_p : process (clk)
  begin
    if rising_edge(clk) then
      p <= to_x01(pull);
    end if;
  end process;

  reg_c : process (clk)
  begin
    if rising_edge(clk) then
      c <= tested;
    end if;
  end process;

  reg_v : process (clk)
  begin
    if rising_edge(clk) then
      qv <= vmid;
    end if;
  end process;

  reg_x : process (clk)
  begin
    if rising_edge(clk) then
      qx <= xmid;
    end if;
  end process;

  reg_h : process (clk)
  begin
    if rising_edge(clk) then
      qh <= (others => half(1));
    end if;
  end process;

  main : process
  begin
    for n in 1 to 8 loop
      if n mod 3 = 0 then
        en <= '0';
      else
        en <= '1';
      end if;
      k <= k + 5;
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      report "k=" & integer'image(n) & " q=" & to_string(q) & " r=" & to_string(r) & " p=" & to_string(p) & " c=" & std_ulogic'image(c)
        & " v=" & to_string(qv) & " x=" & to_string(qx) & " h=" & to_string(qh);
    end loop;
    wait;
  end process;
end architecture;
