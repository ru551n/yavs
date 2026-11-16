-- Memories held in shared variables (A15), `W` replaced by the data width: a single-process RAM
-- that reads before it writes (RBW) and one that writes before it reads (WBR), both read and
-- written at the same and at different addresses on one edge, joining the region with an initial
-- content from a function of a generic; and a RAM written and read by two processes, whose
-- result on a common edge would depend on their order, staying in the event kernel.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shram_sp is
  generic (WBR : boolean; SEED : natural);
  port (clk, we : in std_ulogic; wa, ra : in unsigned(3 downto 0);
        wd : in std_ulogic_vector(W - 1 downto 0); rd : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_shram_sp is
  type mem_t is array (0 to 15) of std_ulogic_vector(W - 1 downto 0);
  function init return mem_t is
    variable m : mem_t;
  begin
    for i in mem_t'range loop
      m(i) := std_ulogic_vector(to_unsigned((i * 37 + SEED) mod 2 ** (W mod 30 + 1), W));
    end loop;
    return m;
  end function;
  shared variable mem : mem_t := init;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if not WBR then
        rd <= mem(to_integer(ra));
      end if;
      if we = '1' then
        mem(to_integer(wa)) := wd;
      end if;
      if WBR then
        rd <= mem(to_integer(ra));
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shram_2p is
  port (clk, clk2, we : in std_ulogic; wa, ra : in unsigned(3 downto 0);
        wd : in std_ulogic_vector(W - 1 downto 0); rd : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_shram_2p is
  type mem_t is array (0 to 15) of std_ulogic_vector(W - 1 downto 0);
  shared variable mem : mem_t := (others => (others => '0'));
begin
  p_write : process (clk)
  begin
    if rising_edge(clk) then
      if we = '1' then
        mem(to_integer(wa)) := wd;
      end if;
    end if;
  end process;
  p_read : process (clk2)
  begin
    if rising_edge(clk2) then
      rd <= mem(to_integer(ra));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
-- Other processes that use the RAM where a scan of names alone would not see it: through an alias,
-- and through a function of the architecture. The RAM's process stays out with them.
entity cycle_shram_hid is
  port (clk, clk2, we : in std_ulogic; wa, ra : in unsigned(3 downto 0);
        wd : in std_ulogic_vector(W - 1 downto 0); rd, pk, fr : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_shram_hid is
  type mem_t is array (0 to 15) of std_ulogic_vector(W - 1 downto 0);
  shared variable mem, mem2 : mem_t := (others => (others => '0'));
  impure function first return std_ulogic_vector is
  begin
    return mem2(0);
  end function;
begin
  p_ram : process (clk)
  begin
    if rising_edge(clk) then
      if we = '1' then
        mem(to_integer(wa)) := wd;
      end if;
      rd <= mem(to_integer(ra));
    end if;
  end process;
  p_ram2 : process (clk)
  begin
    if rising_edge(clk) then
      if we = '1' then
        mem2(to_integer(wa)) := not wd;
      end if;
    end if;
  end process;
  peek : process (clk2)
    alias m : mem_t is mem;
  begin
    if rising_edge(clk2) then
      pk <= m(to_integer(ra));
    end if;
  end process;
  via_fn : process (clk2)
  begin
    if rising_edge(clk2) then
      fr <= first;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shram is
end entity;
architecture t of cycle_shram is
  signal clk, clk2, we : std_ulogic := '0';
  signal wa, ra : unsigned(3 downto 0) := (others => '0');
  signal wd : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal r_rbw, r_wbr, r_2p, r_h, r_pk, r_fr : std_ulogic_vector(W - 1 downto 0);
begin
  rbw : entity work.cycle_shram_sp generic map (WBR => false, SEED => 3)
    port map (clk => clk, we => we, wa => wa, ra => ra, wd => wd, rd => r_rbw);
  wbr : entity work.cycle_shram_sp generic map (WBR => true, SEED => 11)
    port map (clk => clk, we => we, wa => wa, ra => ra, wd => wd, rd => r_wbr);
  tp : entity work.cycle_shram_2p
    port map (clk => clk, clk2 => clk2, we => we, wa => wa, ra => ra, wd => wd, rd => r_2p);
  hid : entity work.cycle_shram_hid
    port map (clk => clk, clk2 => clk2, we => we, wa => wa, ra => ra, wd => wd, rd => r_h, pk => r_pk,
              fr => r_fr);
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 40 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      we <= r(0) or r(5);
      wa <= r(4 downto 1);
      -- Every third edge reads where it writes.
      if k mod 3 = 0 then
        ra <= r(4 downto 1);
      else
        ra <= r(9 downto 6);
      end if;
      wd <= wd(W - 2 downto 0) & r(2) when W > 1 else (0 => r(2));
      wait for 1 ns;
      clk <= '1';
      -- The second clock: an edge with the first every fourth time, else 2 ns later.
      if k mod 4 /= 0 then
        wait for 2 ns;
      end if;
      clk2 <= '1';
      wait for 3 ns;
      clk <= '0';
      clk2 <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " rbw=" & to_hstring(r_rbw) & " wbr=" & to_hstring(r_wbr)
        & " 2p=" & to_hstring(r_2p) & " h=" & to_hstring(r_h) & " pk=" & to_hstring(r_pk)
        & " fr=" & to_hstring(r_fr);
    end loop;
    wait;
  end process;
end architecture;
