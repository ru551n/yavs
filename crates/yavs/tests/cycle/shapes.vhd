-- Clocked and combinational shapes of cycle mode against the normal mode: a `wait until` process
-- with an enable, an asynchronous reset, variables with initial values, an array of vectors read
-- and written at run-time indices, concurrent simple, conditional and selected assignments, a
-- `case` on a vector, an unrolled loop, slices chosen at run time, constants and generics,
-- aggregates of signals, gates of a vector with a scalar, integer `abs`, `minimum`, `maximum`,
-- `numeric_std` with the integer first, a short-circuit `and` guarding an index, `to_x01`,
-- `is_x`, and `numeric_bit` in an entity of its own.
library ieee;
use ieee.numeric_bit.all;
entity cycle_shapes_bit is
  port (
    a : in bit_vector(7 downto 0);
    y : out bit_vector(7 downto 0)
  );
end entity;
architecture rtl of cycle_shapes_bit is
begin
  y <= bit_vector((unsigned(a) + 5) xor shift_left(unsigned(a), 1) xor resize(unsigned(a(3 downto 0)) * 3, 8));
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shapes is
  generic (N : positive := 4; CYCLES : positive := 40);
end entity;
architecture a of cycle_shapes is
  constant K : natural := 3;
  constant MAGIC : unsigned(7 downto 0) := x"A5";
  signal clk, rst : std_ulogic := '0';
  signal din : unsigned(7 downto 0) := (others => '0');
  type arr_t is array (0 to N - 1) of unsigned(7 downto 0);
  signal regs : arr_t := (others => (others => '0'));
  signal acc : unsigned(7 downto 0) := (others => '0');
  signal cnt : integer range 0 to 255 := 0;
  signal en : std_ulogic := '0';
  signal sel : natural range 0 to N - 1 := 0;
  signal pick, mux, sum, dec : unsigned(7 downto 0);
  signal flag : boolean;
  signal code : std_ulogic_vector(1 downto 0);
  signal q : std_ulogic := '0';
  signal wide : std_ulogic_vector(8 * N - 1 downto 0) := (others => '0');
  signal byte_out : std_ulogic_vector(7 downto 0);
  signal agg_a, agg_b, vsg, xo : std_ulogic_vector(7 downto 0);
  signal iabs, imin, imax : integer range -300 to 300;
  signal iv : unsigned(7 downto 0);
  signal sc : std_ulogic;
  signal isx : boolean;
  type pair_t is record
    lo, hi : unsigned(3 downto 0);
  end record;
  signal pr : pair_t;
  signal bin, bout : bit_vector(7 downto 0);
begin
  nb : entity work.cycle_shapes_bit port map (a => bin, y => bout);
  bin <= to_bitvector(std_ulogic_vector(din));
  misc : process (all)
    variable t : integer range -300 to 300;
  begin
    agg_a <= (0 => din(1), 3 => din(0), others => en);
    agg_b <= (din(7), din(6), '1', '0', en, din(2), din(1), din(0));
    vsg <= std_ulogic_vector(din) and en;
    t := to_integer(din) - 128;
    iabs <= abs t;
    imin <= minimum(t, cnt - 100);
    imax <= maximum(t, cnt - 100);
    iv <= 3 + din;
    if sel < N - 1 and regs(sel + 1)(0) = '1' then
      sc <= '1';
    else
      sc <= '0';
    end if;
    xo <= to_x01(std_ulogic_vector(din));
    isx <= is_x(std_ulogic_vector(din));
    pr <= (lo => din(3 downto 0), hi => din(7 downto 4));
  end process;
  w : process
  begin
    wait until rising_edge(clk) and en = '1';
    regs(sel) <= din;
  end process;
  ar : process (clk, rst)
    variable v : unsigned(7 downto 0) := to_unsigned(K, 8);
  begin
    if rst = '1' then
      acc <= (others => '0');
      cnt <= 0;
      v := to_unsigned(K, 8);
    elsif rising_edge(clk) then
      v := v + 1;
      acc <= acc + regs(sel) + v;
      if cnt = 255 then
        cnt <= 0;
      else
        cnt <= cnt + 1;
      end if;
    end if;
  end process;
  q <= din(0) when rising_edge(clk);
  ws : process (clk)
  begin
    if rising_edge(clk) then
      wide(8 * sel + 7 downto 8 * sel) <= std_ulogic_vector(din);
    end if;
  end process;
  byte_out <= wide(8 * sel + 7 downto 8 * sel) xor wide(8 * N - 1 downto 8 * N - 8);
  mux <= regs(0) when sel = 0 else regs(1) xor regs(N - 1) when sel = 1 else not acc;
  pick <= regs(sel);
  flag <= acc > 100;
  with acc(1 downto 0) select code <= "11" when "00", "10" when "01", "01" when others;
  sums : process (regs)
    variable s : unsigned(7 downto 0);
  begin
    s := MAGIC;
    for i in regs'range loop
      s := s xor regs(i);
    end loop;
    sum <= s;
  end process;
  decode : process (all)
  begin
    case std_ulogic_vector(acc(7 downto 6)) is
      when "00" => dec <= acc;
      when "01" => dec <= acc + 1;
      when "10" => dec <= shift_right(acc, 1);
      when others => dec <= (others => '1');
    end case;
  end process;
  main : process
    variable x : unsigned(15 downto 0) := x"ACE1";
  begin
    for c in 1 to CYCLES loop
      x := x(14 downto 0) & (x(15) xor x(13) xor x(12) xor x(10));
      din <= x(7 downto 0);
      en <= x(9);
      sel <= to_integer(x(12 downto 11)) mod N;
      rst <= '1' when c = 3 or c = 17 else '0';
      wait for 2 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 3 ns;
      report "c=" & integer'image(c) & " acc=" & to_hstring(acc) & " cnt=" & integer'image(cnt)
        & " mux=" & to_hstring(mux) & " pick=" & to_hstring(pick) & " flag=" & boolean'image(flag)
        & " code=" & to_string(code) & " sum=" & to_hstring(sum) & " dec=" & to_hstring(dec)
        & " q=" & std_ulogic'image(q) & " r0=" & to_hstring(regs(0)) & " r3=" & to_hstring(regs(N - 1))
        & " wide=" & to_hstring(wide) & " bo=" & to_hstring(byte_out) & " aa=" & to_hstring(agg_a)
        & " ab=" & to_hstring(agg_b) & " vsg=" & to_hstring(vsg) & " ia=" & integer'image(iabs)
        & " imn=" & integer'image(imin) & " imx=" & integer'image(imax) & " iv=" & to_hstring(iv)
        & " sc=" & std_ulogic'image(sc) & " xo=" & to_hstring(xo) & " isx=" & boolean'image(isx)
        & " pr=" & to_hstring(pr.hi) & to_hstring(pr.lo) & " bout=" & to_hstring(bout);
    end loop;
    wait;
  end process;
end architecture;
