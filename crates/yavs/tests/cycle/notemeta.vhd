-- Asserts and reports under `--cycle-exact-meta` read the second image: registers with a 'U' or
-- 'X' start (whole, in a record field, in a part of a vector; widths 1/63/64/65/128) print as in
-- the normal mode at the first edge, from a clocked checker of the top-level unit and from a
-- combinational process below it. A BIT copy of a 'U' stays 0. `zero` fires on the region's
-- image only (q1 reads 0 there, the normal mode does not fire): two-state text; `one` fires on
-- the second image only: nothing. `comb`'s report of the event from 'U' to 0 at 0 fs is gone.
library ieee;
use ieee.std_logic_1164.all;

package cycle_notemeta_pkg is
  type rec_t is record
    a : std_logic_vector(64 downto 0);
    n : integer;
    b : std_logic;
  end record;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_notemeta_pkg.all;

entity cycle_notemeta_dut is
  port (
    clk  : in std_logic;
    d    : in std_logic;
    q1   : out std_logic;
    qx   : out std_logic;
    q63  : out std_logic_vector(62 downto 0);
    q64  : out std_logic_vector(63 downto 0);
    q65  : out std_logic_vector(64 downto 0);
    q128 : out std_logic_vector(127 downto 0);
    p128 : out std_logic_vector(127 downto 0);
    r    : out rec_t;
    bv   : out bit_vector(3 downto 0));
end;

architecture a of cycle_notemeta_dut is
  signal s1   : std_logic;
  signal sx   : std_logic := 'X';
  signal s63  : std_logic_vector(62 downto 0);
  signal s64  : std_logic_vector(63 downto 0);
  signal s65  : std_logic_vector(64 downto 0);
  signal s128 : std_logic_vector(127 downto 0);
  signal sp   : std_logic_vector(127 downto 0) :=
    (127 downto 120 => 'U', 70 downto 60 => 'X', 3 => 'U', others => '0');
  signal sr   : rec_t;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      s1   <= d;
      sx   <= d;
      s63  <= (others => d);
      s64  <= (others => d);
      s65  <= (others => d);
      s128 <= (others => d);
      sp   <= (others => d);
      sr   <= ((others => d), 7, d);
      bv   <= to_bitvector(s64(3 downto 0));
    end if;
  end process;
  q1 <= s1; qx <= sx; q63 <= s63; q64 <= s64; q65 <= s65; q128 <= s128; p128 <= sp; r <= sr;

  comb : process (s1, sx, s65)
  begin
    report "comb s1=" & to_string(s1) & " sx=" & to_string(sx) & " s65=" & to_string(s65);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_notemeta_pkg.all;

entity cycle_notemeta is
end;

architecture a of cycle_notemeta is
  signal clk, d : std_logic := '0';
  signal q1, qx : std_logic;
  signal q63    : std_logic_vector(62 downto 0);
  signal q64    : std_logic_vector(63 downto 0);
  signal q65    : std_logic_vector(64 downto 0);
  signal q128   : std_logic_vector(127 downto 0);
  signal p128   : std_logic_vector(127 downto 0);
  signal r      : rec_t;
  signal bv     : bit_vector(3 downto 0);
begin
  dut : entity work.cycle_notemeta_dut port map (clk, d, q1, qx, q63, q64, q65, q128, p128, r, bv);

  chk : process (clk)
  begin
    if rising_edge(clk) then
      report "q1=" & to_string(q1) & " qx=" & std_logic'image(qx) & " q63=" & to_string(q63);
      report "q64=" & to_hstring(q64) & " q65=" & to_string(q65);
      report "q128=" & to_string(q128) & " p128=" & to_string(p128);
      report "ph=" & to_hstring(p128) & " po=" & to_ostring(p128(11 downto 0));
      report "r.a=" & to_string(r.a) & " r.n=" & integer'image(r.n) & " r.b=" & to_string(r.b);
      report "bv=" & to_string(bv) & " eq=" & boolean'image(q1 = '1');
      if q1 = '0' then
        report "zero q64=" & to_hstring(q64);
      end if;
      if q1 = '1' then
        report "one q64=" & to_hstring(q64);
      end if;
    end if;
  end process;

  process
  begin
    for i in 1 to 3 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      d <= not d;
    end loop;
    wait;
  end process;
end;
