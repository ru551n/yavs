-- `--init-unset`: registers with neither reset nor initial value (a counter, a record field, a
-- clocked variable) at widths 1, 63, 64, 65 and 128. The checks pass only when they start at 0.
-- A run-time package, and an elaboration-time call with package state: the lowering of the
-- design goes on from the session's.
package iu_pkg is
  impure function next_id return natural;
end package;

package body iu_pkg is
  type counter_t is protected
    impure function next_id return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
  shared variable ids : counter_t;
  impure function next_id return natural is
  begin
    return ids.next_id;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity iu_reg is
  generic (W : positive);
  port (
    clk : in std_logic;
    q, r, v : out std_logic_vector(W - 1 downto 0);
    t : out std_logic_vector(4 * W downto 0);
    n : out integer range 0 to 7;
    s, wr : out std_logic);
end entity;

architecture rtl of iu_reg is
  signal cnt : unsigned(W - 1 downto 0);
  type rec_t is record
    a : std_ulogic_vector(W - 1 downto 0);
    n : integer range 0 to 7;
  end record;
  signal rec : rec_t;
  signal bit1 : std_ulogic;
  -- Resolved, with two drivers: both start at its start value.
  signal w2 : std_logic;
  -- With an initial value: never changed by the option, and not listed.
  signal keep : std_ulogic := '1';
begin
  process (clk) begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
      rec.a <= not rec.a;
      rec.n <= (rec.n + 1) mod 8;
      bit1 <= bit1 xor keep;
      keep <= not keep;
    end if;
  end process;
  q <= std_logic_vector(cnt);
  r <= std_logic_vector(rec.a);
  n <= rec.n;
  s <= bit1;
  process (clk) begin
    if rising_edge(clk) then
      w2 <= w2;
    end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then
      w2 <= w2;
    end if;
  end process;
  wr <= w2;
  -- Clocked variables without an initial value: a vector, a scalar, a record, an array of vectors.
  process (clk)
    variable acc : std_ulogic_vector(W - 1 downto 0);
    variable sc : std_ulogic;
    variable rv : rec_t;
    type mem_t is array (0 to 2) of std_ulogic_vector(W - 1 downto 0);
    variable mem : mem_t;
  begin
    if rising_edge(clk) then
      acc := not acc;
      sc := not sc;
      rv.a := not rv.a;
      mem(1) := not mem(1);
      v <= std_logic_vector(acc);
      t <= sc & std_logic_vector(rv.a) & std_logic_vector(mem(0)) & std_logic_vector(mem(1))
        & std_logic_vector(mem(2));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity initunset is
end entity;

architecture tb of initunset is
  constant ID : natural := work.iu_pkg.next_id;
  type widths_t is array (0 to 4) of positive;
  constant WIDTHS : widths_t := (1, 63, 64, 65, 128);
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  g : for i in WIDTHS'range generate
    constant W : positive := WIDTHS(i);
    signal q, r, v : std_logic_vector(W - 1 downto 0);
    signal t : std_logic_vector(4 * W downto 0);
    signal n : integer range 0 to 7;
    signal s, wr : std_logic;
  begin
    dut : entity work.iu_reg generic map (W) port map (clk, q, r, v, t, n, s, wr);
    process begin
      for k in 1 to 5 loop
        wait until rising_edge(clk);
      end loop;
      wait for 1 ns;
      report "W=" & integer'image(W) & " q=" & to_hstring(q) & " r=" & to_hstring(r)
        & " v=" & to_hstring(v) & " t=" & to_hstring(t) & " n=" & integer'image(n)
        & " s=" & std_logic'image(s) & " w=" & std_logic'image(wr);
      assert q = std_logic_vector(resize(to_unsigned(5, 8), W))
        and r = (r'range => '1') and v = (v'range => '1')
        report "W=" & integer'image(W) & ": did not start at 0" severity error;
      wait;
    end process;
  end generate;
  process begin
    wait for 100 ns;
    std.env.finish;
  end process;
end architecture;
