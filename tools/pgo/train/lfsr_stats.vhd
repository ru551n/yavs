-- pgo: top=lfsr_stats_tb std=2008 stop=4ms
-- Configurable LFSR generators (width, taps, seed, output width) whose output
-- is checked statistically: ones count, run lengths, serial correlation,
-- entropy, period. Real-number maths and hand formatted reports.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;

package lfsr_pkg is
  function poly_degrees (width : positive; sel : natural) return integer_vector;
  function tap_mask (width : positive; sel : natural) return std_logic_vector;
  function parity (v : std_logic_vector) return std_logic;
  function xor_masked (v : std_logic_vector; mask : std_logic_vector) return std_logic;
  function reverse_bits (v : std_logic_vector) return std_logic_vector;
  function ones_count (v : std_logic_vector) return natural;
  function lfsr_next (state : std_logic_vector; mask : std_logic_vector)
    return std_logic_vector;
  function fixed (r : real; digits : positive) return string;
end package lfsr_pkg;

package body lfsr_pkg is
  -- Degrees of a primitive polynomial, zero meaning unused.
  function poly_degrees (width : positive; sel : natural) return integer_vector is
  begin
    case width is
      when 5 =>
        if sel = 0 then return (5, 3, 0, 0); else return (5, 2, 0, 0); end if;
      when 8 =>
        if sel = 0 then return (8, 6, 5, 4); else return (8, 7, 6, 1); end if;
      when 10 =>
        if sel = 0 then return (10, 7, 0, 0); else return (10, 3, 0, 0); end if;
      when 12 =>
        if sel = 0 then return (12, 11, 10, 4); else return (12, 6, 4, 1); end if;
      when 16 =>
        if sel = 0 then return (16, 15, 13, 4); else return (16, 14, 13, 11); end if;
      when others =>
        report "unsupported width " & integer'image(width) severity failure;
        return (0, 0, 0, 0);
    end case;
  end function;

  function tap_mask (width : positive; sel : natural) return std_logic_vector is
    constant degs : integer_vector := poly_degrees(width, sel);
    variable m : std_logic_vector(width - 1 downto 0) := (others => '0');
  begin
    for i in degs'range loop
      if degs(i) > 0 then
        m(degs(i) - 1) := '1';
      end if;
    end loop;
    return m;
  end function;

  function parity (v : std_logic_vector) return std_logic is
    variable p : std_logic := '0';
  begin
    for i in v'range loop
      p := p xor v(i);
    end loop;
    return p;
  end function;

  function xor_masked (v : std_logic_vector; mask : std_logic_vector) return std_logic is
  begin
    return parity(v and mask);
  end function;

  function reverse_bits (v : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(v'range);
  begin
    for i in v'range loop
      r(v'left + v'right - i) := v(i);
    end loop;
    return r;
  end function;

  function ones_count (v : std_logic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  -- One shift of a Fibonacci register, new bit in at the low end.
  function lfsr_next (state : std_logic_vector; mask : std_logic_vector)
    return std_logic_vector is
  begin
    return state(state'left - 1 downto state'right) & xor_masked(state, mask);
  end function;

  function zpad (s : string; n : natural) return string is
    variable r : string(1 to maximum(s'length, n)) := (others => '0');
  begin
    r(r'right - s'length + 1 to r'right) := s;
    return r;
  end function;

  -- Fixed point text of a real without using real'image.
  function fixed (r : real; digits : positive) return string is
    constant scale : integer := 10 ** digits;
    constant scaled : integer := integer(round(abs(r) * real(scale)));
  begin
    if r < 0.0 then
      return "-" & integer'image(scaled / scale) & "." & zpad(integer'image(scaled mod scale), digits);
    end if;
    return integer'image(scaled / scale) & "." & zpad(integer'image(scaled mod scale), digits);
  end function;
end package body lfsr_pkg;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.lfsr_pkg.all;

entity lfsr_gen is
  generic (
    width       : positive := 8;
    taps_sel    : natural := 0;
    seed        : positive := 1;
    out_width   : positive := 8;
    reverse_out : boolean := false
  );
  port (
    clk     : in  std_logic;
    rst     : in  std_logic;
    en      : in  std_logic;
    state   : out std_logic_vector(width - 1 downto 0);
    dout    : out std_logic_vector(out_width - 1 downto 0);
    bit_out : out std_logic
  );
end entity lfsr_gen;

architecture rtl of lfsr_gen is
  constant MASK : std_logic_vector(width - 1 downto 0) := tap_mask(width, taps_sel);
  constant INIT : std_logic_vector(width - 1 downto 0) :=
    std_logic_vector(to_unsigned(seed mod 2 ** width, width));
  signal s : std_logic_vector(width - 1 downto 0) := INIT;
begin
  assert out_width <= width
    report "output wider than the register" severity failure;
  assert unsigned(INIT) /= 0
    report "seed is zero modulo the register size" severity failure;

  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        s <= INIT;
      elsif en = '1' then
        s <= lfsr_next(s, MASK);
      end if;
    end if;
  end process;

  state   <= s;
  bit_out <= s(0);

  g_rev : if reverse_out generate
    dout <= reverse_bits(s(width - 1 downto width - out_width));
  else generate
    dout <= s(width - 1 downto width - out_width);
  end generate g_rev;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;
use work.lfsr_pkg.all;

-- One generator plus a process that gathers statistics of its output.
entity lfsr_check is
  generic (
    tag         : string;
    width       : positive;
    taps_sel    : natural;
    seed        : positive;
    out_width   : positive;
    reverse_out : boolean;
    nbits       : positive
  );
  port (
    clk      : in  std_logic;
    finished : out std_logic
  );
end entity lfsr_check;

architecture sim of lfsr_check is
  constant INIT : std_logic_vector(width - 1 downto 0) :=
    std_logic_vector(to_unsigned(seed mod 2 ** width, width));
  constant BINS : positive := 2 ** out_width;
  signal rst, en : std_logic := '0';
  signal st   : std_logic_vector(width - 1 downto 0);
  signal word : std_logic_vector(out_width - 1 downto 0);
  signal b    : std_logic;
begin
  dut : entity work.lfsr_gen
    generic map (width => width, taps_sel => taps_sel, seed => seed,
                 out_width => out_width, reverse_out => reverse_out)
    port map (clk => clk, rst => rst, en => en, state => st, dout => word, bit_out => b);

  stats : process
    type hist_t is array (0 to BINS - 1) of natural;
    type run_t is array (1 to 16) of natural;
    constant degs : integer_vector := poly_degrees(width, taps_sel);
    variable hist : hist_t := (others => 0);
    variable runs0, runs1 : run_t := (others => 0);
    variable model : std_logic_vector(width - 1 downto 0) := INIT;
    variable fb : std_logic;
    variable bit_v, prev, ones, run_len, max_run : natural := 0;
    variable sx, sy, sxy : natural := 0;
    variable total0, total1, widx : natural;
    variable fn, corr, num, den, ent, p, chi, e, d : real;
  begin
    rst <= '1';
    en <= '0';
    wait until rising_edge(clk);
    wait until rising_edge(clk);
    rst <= '0';
    en <= '1';
    for k in 1 to nbits loop
      wait until rising_edge(clk);
      -- compare against a model that walks the polynomial degrees
      assert st = model
        report tag & ": state mismatch at step " & integer'image(k) severity failure;
      fb := '0';
      for j in degs'range loop
        if degs(j) > 0 then
          fb := fb xor model(degs(j) - 1);
        end if;
      end loop;
      model := model(width - 2 downto 0) & fb;

      if b = '1' then bit_v := 1; else bit_v := 0; end if;
      ones := ones + bit_v;
      hist(to_integer(unsigned(word))) := hist(to_integer(unsigned(word))) + 1;
      if k = 1 then
        run_len := 1;
      else
        sx := sx + prev;
        sy := sy + bit_v;
        sxy := sxy + prev * bit_v;
        if bit_v = prev then
          run_len := run_len + 1;
        else
          widx := minimum(run_len, 16);
          if prev = 1 then
            runs1(widx) := runs1(widx) + 1;
          else
            runs0(widx) := runs0(widx) + 1;
          end if;
          max_run := maximum(max_run, run_len);
          run_len := 1;
        end if;
      end if;
      prev := bit_v;
    end loop;
    en <= '0';

    total0 := 0;
    total1 := 0;
    for i in run_t'range loop
      total0 := total0 + runs0(i);
      total1 := total1 + runs1(i);
    end loop;
    fn := real(nbits);

    -- balance of ones and zeros within five standard deviations, plus the
    -- one extra one per period of a maximal register
    assert abs(2 * ones - nbits) <= integer(5.0 * sqrt(fn)) + nbits / (2 ** width - 1) + 2
      report tag & ": ones count " & integer'image(ones) severity failure;
    -- about half of all runs are of length one, and there are n/2 runs
    assert abs(total0 + total1 - nbits / 2) <= nbits / 25
      report tag & ": run count " & integer'image(total0 + total1) severity failure;
    assert abs(2 * runs1(1) - total1) * 10 <= total1 and abs(2 * runs0(1) - total0) * 10 <= total0
      report tag & ": runs of length one" severity failure;
    assert max_run <= width
      report tag & ": longest run " & integer'image(max_run) severity failure;

    -- lag one serial correlation
    num := real(nbits - 1) * real(sxy) - real(sx) * real(sy);
    den := sqrt((real(nbits - 1) * real(sx) - real(sx) ** 2)
              * (real(nbits - 1) * real(sy) - real(sy) ** 2));
    corr := num / den;
    -- a full period has a small built-in negative correlation of 1/(2^w - 1)
    assert abs(corr) < 5.0 / sqrt(fn) + 2.0 / real(2 ** width - 1)
      report tag & ": serial correlation " & fixed(corr, 4) severity failure;

    -- entropy and chi square over the output words
    ent := 0.0;
    chi := 0.0;
    for i in hist_t'range loop
      -- the all-zero state never occurs, so bin zero has one state less
      if i = 0 then
        e := fn * real(2 ** (width - out_width) - 1) / real(2 ** width - 1);
      else
        e := fn * real(2 ** (width - out_width)) / real(2 ** width - 1);
      end if;
      d := real(hist(i)) - e;
      chi := chi + d * d / e;
      if hist(i) > 0 then
        p := real(hist(i)) / fn;
        ent := ent - p * log2(p);
      end if;
    end loop;
    assert ent >= 0.97 * real(out_width)
      report tag & ": entropy " & fixed(ent, 3) severity failure;
    assert chi <= real(BINS) + 10.0 * sqrt(2.0 * real(BINS)) + 20.0
      report tag & ": chi square " & fixed(chi, 1) severity failure;

    report tag & ": n=" & integer'image(nbits) & " ones=" & integer'image(ones)
           & " runs=" & integer'image(total0 + total1) & " maxrun=" & integer'image(max_run)
           & " corr=" & fixed(corr, 4) & " entropy=" & fixed(ent, 3) & " chi=" & fixed(chi, 1);
    finished <= '1';
    wait;
  end process stats;
end architecture sim;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;
use work.lfsr_pkg.all;

entity lfsr_stats_tb is
end entity lfsr_stats_tb;

architecture sim of lfsr_stats_tb is
  constant WIDTHS : integer_vector(0 to 4) := (5, 8, 10, 12, 16);
  constant N_INST : natural := 10;

  signal clk : std_logic := '0';
  signal fin : std_logic_vector(0 to N_INST - 1) := (others => '0');
  signal all_fin : std_logic;
  signal per_rst, per_en : std_logic := '0';
  signal per_state : std_logic_vector(7 downto 0);
  signal per_dout  : std_logic_vector(7 downto 0);
  signal per_bit   : std_logic;

  function bits_for (w : positive) return positive is
  begin
    if w >= 16 then
      return 100000;
    elsif w >= 10 then
      return 60000;
    end if;
    return 30000;
  end function;

  -- Period of a register found by stepping until the seed returns.
  function period_of (width : positive; sel : natural) return natural is
    constant mask : std_logic_vector(width - 1 downto 0) := tap_mask(width, sel);
    variable s0, s : std_logic_vector(width - 1 downto 0) := (0 => '1', others => '0');
    variable n : natural := 0;
  begin
    loop
      s := lfsr_next(s, mask);
      n := n + 1;
      exit when s = s0 or n > 2 ** width;
    end loop;
    return n;
  end function;
begin
  clk <= not clk after 5 ns;

  g_sel : for sel in 0 to 1 generate
    g_w : for k in 0 to 4 generate
      constant W   : positive := WIDTHS(k);
      constant IDX : natural := sel * 5 + k;
    begin
      u_chk : entity work.lfsr_check
        generic map (
          tag         => "w" & integer'image(W) & "s" & integer'image(sel),
          width       => W,
          taps_sel    => sel,
          seed        => 1 + 37 * IDX,
          out_width   => minimum(8, W - 2),
          reverse_out => k mod 2 = 1,
          nbits       => bits_for(W))
        port map (clk => clk, finished => fin(IDX));
    end generate g_w;
  end generate g_sel;

  all_fin <= '1' when fin = (fin'range => '1') else '0';

  per_dut : entity work.lfsr_gen
    generic map (width => 8, taps_sel => 0, seed => 1, out_width => 8)
    port map (clk => clk, rst => per_rst, en => per_en, state => per_state,
              dout => per_dout, bit_out => per_bit);

  main : process
    variable count : natural;
    variable s : std_logic_vector(15 downto 0) := x"ACE1";
    variable m16 : std_logic_vector(15 downto 0) := tap_mask(16, 0);
    variable r : std_logic_vector(15 downto 0);
    variable ones_total : natural := 0;
  begin
    wait until all_fin = '1' for 3 ms;
    assert all_fin = '1' report "statistics checks did not finish" severity failure;

    -- period of the register itself, from its state output
    per_rst <= '1';
    wait until rising_edge(clk);
    wait until rising_edge(clk);
    per_rst <= '0';
    per_en <= '1';
    count := 0;
    loop
      wait until rising_edge(clk);
      if count > 0 and per_state = x"01" then
        exit;
      end if;
      count := count + 1;
      assert count < 300 report "no period found" severity failure;
    end loop;
    assert count = 255 report "period " & integer'image(count) severity failure;
    per_en <= '0';

    -- period of every supported register by function stepping
    for k in 0 to 3 loop
      for sel in 0 to 1 loop
        count := period_of(WIDTHS(k), sel);
        assert count = 2 ** WIDTHS(k) - 1
          report "period of width " & integer'image(WIDTHS(k)) & " is " & integer'image(count)
          severity failure;
      end loop;
    end loop;
    count := period_of(16, 0);
    assert count = 65535 report "period 16" severity failure;

    -- helper functions on slices
    for i in 1 to 400 loop
      s := lfsr_next(s, m16);
      r := reverse_bits(s);
      assert reverse_bits(r) = s report "reverse twice" severity failure;
      assert r(15) = s(0) and r(0) = s(15) and r(8 downto 7) = s(7) & s(8)
        report "reverse position" severity failure;
      assert parity(s) = std_logic'val(ones_count(s) mod 2 + 2)
        report "parity vs count" severity failure;
      assert xor_masked(s, x"00FF") = parity(s(7 downto 0)) report "masked xor" severity failure;
      ones_total := ones_total + ones_count(s);
    end loop;
    report "mean ones per word x100: " & integer'image(ones_total * 100 / 400);
    assert ones_total > 400 * 6 and ones_total < 400 * 10 report "word balance" severity failure;

    -- text formatting without real'image
    assert fixed(3.14159, 2) = "3.14" report fixed(3.14159, 2) severity failure;
    assert fixed(-0.05, 3) = "-0.050" report fixed(-0.05, 3) severity failure;
    assert fixed(12.0, 1) = "12.0" report fixed(12.0, 1) severity failure;

    report "PASS" severity note;
    wait;
  end process main;
end architecture sim;
