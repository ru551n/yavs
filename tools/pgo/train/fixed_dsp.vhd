-- pgo: top=fixed_dsp_tb std=2008 stop=20ms
-- Reproduces fixed-point signal processing blocks and their reference checks: a MAC unit, a FIR
-- filter, a rotation-mode CORDIC pipeline, a division by repeated subtraction and a sine table
-- in RAM, all checked against real arithmetic from math_real.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;

package fx_pkg is
  subtype q15_t is signed(15 downto 0);
  subtype acc_t is signed(39 downto 0);
  subtype wide_t is signed(23 downto 0);
  type q15_vec_t is array (natural range <>) of q15_t;
  type rom_t is array (0 to 255) of q15_t;
  constant n_taps : positive := 15;
  constant n_stages : positive := 16;
  type atan_tab_t is array (0 to n_stages - 1) of wide_t;

  function sat (x : signed; width : positive) return signed;
  function round_shift (x : signed; n : positive) return signed;
  function to_q15 (r : real) return q15_t;
  function from_q15 (q : signed) return real;
  function init_coefs return q15_vec_t;
  function init_sin_rom return rom_t;
  function atan_table return atan_tab_t;
  function cordic_gain return real;
  function sample_at (n : natural) return q15_t;
  function angle_q (k, count : natural) return wide_t;
  function next_rand (x : natural) return natural;
end package fx_pkg;

package body fx_pkg is
  -- Saturate to width bits (width must be smaller than x'length).
  function sat (x : signed; width : positive) return signed is
    constant hi : signed(x'length - 1 downto 0) := to_signed(2 ** (width - 1) - 1, x'length);
    constant lo : signed(x'length - 1 downto 0) := to_signed(-(2 ** (width - 1)), x'length);
  begin
    if x > hi then
      return resize(hi, width);
    elsif x < lo then
      return resize(lo, width);
    end if;
    return resize(x, width);
  end function;

  -- Arithmetic right shift with round half up.
  function round_shift (x : signed; n : positive) return signed is
    constant w : signed(x'length downto 0) := resize(x, x'length + 1) + to_signed(2 ** (n - 1), x'length + 1);
  begin
    return resize(shift_right(w, n), x'length);
  end function;

  function to_q15 (r : real) return q15_t is
    variable i : integer := integer(round(r * 32768.0));
  begin
    if i > 32767 then
      i := 32767;
    elsif i < -32768 then
      i := -32768;
    end if;
    return to_signed(i, 16);
  end function;

  function from_q15 (q : signed) return real is
  begin
    return real(to_integer(q)) / 32768.0;
  end function;

  -- Hamming windowed sinc low-pass, cut-off 0.2 of the sample rate, unity gain at dc.
  function init_coefs return q15_vec_t is
    variable h : q15_vec_t(0 to n_taps - 1);
    variable v : real;
    variable x : real;
    variable total : real := 0.0;
    type real_arr_t is array (0 to n_taps - 1) of real;
    variable raw : real_arr_t;
  begin
    for k in 0 to n_taps - 1 loop
      x := real(k - (n_taps - 1) / 2);
      if k = (n_taps - 1) / 2 then
        v := 0.4;
      else
        v := sin(2.0 * math_pi * 0.2 * x) / (math_pi * x);
      end if;
      raw(k) := v * (0.54 - 0.46 * cos(2.0 * math_pi * real(k) / real(n_taps - 1)));
      total := total + raw(k);
    end loop;
    for k in 0 to n_taps - 1 loop
      h(k) := to_q15(raw(k) / total);
    end loop;
    return h;
  end function;

  function init_sin_rom return rom_t is
    variable r : rom_t;
  begin
    for i in r'range loop
      r(i) := to_q15(sin(2.0 * math_pi * real(i) / 256.0));
    end loop;
    return r;
  end function;

  function atan_table return atan_tab_t is
    variable t : atan_tab_t;
  begin
    for i in t'range loop
      t(i) := to_signed(integer(round(arctan(2.0 ** (-i)) * 65536.0)), 24);
    end loop;
    return t;
  end function;

  function cordic_gain return real is
    variable k : real := 1.0;
  begin
    for i in 0 to n_stages - 1 loop
      k := k / sqrt(1.0 + 2.0 ** (-2 * i));
    end loop;
    return k;
  end function;

  -- Test signal: a slow and a fast sine, so that the filter has something to remove.
  function sample_at (n : natural) return q15_t is
  begin
    return to_q15(0.5 * sin(2.0 * math_pi * 0.05 * real(n)) + 0.25 * sin(2.0 * math_pi * 0.37 * real(n)));
  end function;

  function angle_q (k, count : natural) return wide_t is
  begin
    return to_signed(integer(round((-math_pi + 2.0 * math_pi * real(k) / real(count)) * 65536.0)), 24);
  end function;

  function next_rand (x : natural) return natural is
  begin
    return (x * 75 + 74) mod 65537;
  end function;
end package body fx_pkg;

library ieee;
use ieee.numeric_std.all;
use work.fx_pkg.all;

-- Constant tables built by functions of fx_pkg.
package fx_tables is
  constant coef_rom : q15_vec_t(0 to n_taps - 1) := init_coefs;
  constant sin_rom : rom_t := init_sin_rom;
end package fx_tables;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.fx_pkg.all;

-- Multiply-accumulate with a 40 bit accumulator and a rounded, saturated 16 bit result.
entity mac_unit is
  port (
    clk : in std_logic;
    clear, en : in std_logic;
    a, b : in q15_t;
    acc : out acc_t;
    result : out q15_t;
    ovf : out std_logic
  );
end entity mac_unit;

architecture rtl of mac_unit is
  signal acc_r : acc_t := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if clear = '1' then
        acc_r <= (others => '0');
      elsif en = '1' then
        acc_r <= acc_r + resize(a * b, acc_r'length);
      end if;
    end if;
  end process;
  acc <= acc_r;
  result <= sat(round_shift(acc_r, 15), 16);
  ovf <= '1' when acc_r > to_signed(2 ** 30 - 1, 40) or acc_r < to_signed(-(2 ** 30), 40) else '0';
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.fx_pkg.all;

-- Direct form FIR: shift register, parallel products from a generate loop, rounded sum.
entity fir_filter is
  generic (taps : positive := n_taps);
  port (
    clk : in std_logic;
    coefs : in q15_vec_t(0 to taps - 1);
    in_valid : in std_logic;
    in_sample : in q15_t;
    out_valid : out std_logic;
    out_sample : out q15_t
  );
end entity fir_filter;

architecture rtl of fir_filter is
  type prod_vec_t is array (0 to taps - 1) of signed(31 downto 0);
  signal dly : q15_vec_t(0 to taps - 1) := (others => (others => '0'));
  signal prod : prod_vec_t;
  signal acc : acc_t;
  signal v_a : std_logic := '0';
begin
  shift : process (clk)
  begin
    if rising_edge(clk) then
      v_a <= in_valid;
      if in_valid = '1' then
        dly <= in_sample & dly(0 to taps - 2);
      end if;
    end if;
  end process;

  mults : for i in 0 to taps - 1 generate
    prod(i) <= coefs(i) * dly(i);
  end generate;

  sum : process (all)
    variable s : acc_t;
  begin
    s := (others => '0');
    for i in 0 to taps - 1 loop
      s := s + resize(prod(i), s'length);
    end loop;
    acc <= s;
  end process;

  outp : process (clk)
  begin
    if rising_edge(clk) then
      out_valid <= v_a;
      out_sample <= sat(round_shift(acc, 15), 16);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;
use work.fx_pkg.all;

-- Rotation mode CORDIC: a quadrant fold stage, then one pipeline register per iteration.
entity cordic_rot is
  generic (stages : positive := n_stages);
  port (
    clk : in std_logic;
    in_valid : in std_logic;
    angle : in wide_t;
    out_valid : out std_logic;
    cos_o, sin_o : out wide_t
  );
end entity cordic_rot;

architecture pipe of cordic_rot is
  type stage_t is record
    x, y, z : wide_t;
    v : std_logic;
  end record;
  type stage_arr_t is array (natural range <>) of stage_t;
  constant zero_stage : stage_t := (x => (others => '0'), y => (others => '0'), z => (others => '0'), v => '0');
  constant atan_tab : atan_tab_t := atan_table;
  constant half_pi : wide_t := to_signed(integer(round(math_pi / 2.0 * 65536.0)), 24);
  constant k0 : wide_t := to_signed(integer(round(cordic_gain * 65536.0)), 24);
  signal st : stage_arr_t(0 to stages) := (others => zero_stage);
begin
  fold : process (clk)
  begin
    if rising_edge(clk) then
      st(0).v <= in_valid;
      if angle > half_pi then
        st(0).x <= (others => '0');
        st(0).y <= k0;
        st(0).z <= angle - half_pi;
      elsif angle < -half_pi then
        st(0).x <= (others => '0');
        st(0).y <= -k0;
        st(0).z <= angle + half_pi;
      else
        st(0).x <= k0;
        st(0).y <= (others => '0');
        st(0).z <= angle;
      end if;
    end if;
  end process;

  iter : for i in 0 to stages - 1 generate
    process (clk)
      variable dx, dy : wide_t;
    begin
      if rising_edge(clk) then
        dx := shift_right(st(i).y, i);
        dy := shift_right(st(i).x, i);
        if st(i).z >= 0 then
          st(i + 1).x <= st(i).x - dx;
          st(i + 1).y <= st(i).y + dy;
          st(i + 1).z <= st(i).z - atan_tab(i);
        else
          st(i + 1).x <= st(i).x + dx;
          st(i + 1).y <= st(i).y - dy;
          st(i + 1).z <= st(i).z + atan_tab(i);
        end if;
        st(i + 1).v <= st(i).v;
      end if;
    end process;
  end generate iter;

  out_valid <= st(stages).v;
  cos_o <= st(stages).x;
  sin_o <= st(stages).y;
end architecture pipe;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Divider by repeated subtraction: one subtraction per clock, so slow for big quotients.
entity div_sub is
  generic (width : positive := 16);
  port (
    clk : in std_logic;
    start : in std_logic;
    num, den : in unsigned(width - 1 downto 0);
    quot, rem_out : out unsigned(width - 1 downto 0);
    done, div0 : out std_logic
  );
end entity div_sub;

architecture fsm of div_sub is
  type state_t is (idle, run, fin);
  signal state : state_t := idle;
  signal r, q, d : unsigned(width - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      done <= '0';
      case state is
        when idle =>
          if start = '1' then
            if den = 0 then
              div0 <= '1';
              quot <= (others => '1');
              rem_out <= num;
              done <= '1';
            else
              div0 <= '0';
              r <= num;
              d <= den;
              q <= (others => '0');
              state <= run;
            end if;
          end if;
        when run =>
          if r >= d then
            r <= r - d;
            q <= q + 1;
          else
            state <= fin;
          end if;
        when fin =>
          quot <= q;
          rem_out <= r;
          done <= '1';
          state <= idle;
      end case;
    end if;
  end process;
end architecture fsm;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;
use work.fx_pkg.all;
use work.fx_tables.all;

entity fixed_dsp_tb is
end entity fixed_dsp_tb;

architecture tb of fixed_dsp_tb is
  constant n_fir : natural := 22000;
  constant n_angles : natural := 22000;
  constant n_mac : natural := 500;
  constant n_div : natural := 400;
  constant n_ram : natural := 256;

  signal clk : std_logic := '0';
  signal finished : boolean := false;

  -- MAC
  signal mac_clear, mac_en : std_logic := '0';
  signal mac_a, mac_b : q15_t := (others => '0');
  signal mac_acc : acc_t;
  signal mac_res : q15_t;
  signal mac_ovf : std_logic;
  signal mac_done : boolean := false;

  -- FIR
  signal fir_valid, fir_out_valid : std_logic := '0';
  signal fir_in, fir_out : q15_t := (others => '0');
  signal fir_done : boolean := false;

  -- CORDIC
  signal cor_valid, cor_out_valid : std_logic := '0';
  signal cor_angle, cor_cos, cor_sin : wide_t := (others => '0');
  signal cor_done : boolean := false;

  -- divider
  signal div_start, div_done, div_by0 : std_logic := '0';
  signal div_num, div_den, div_q, div_r : unsigned(15 downto 0) := (others => '0');
  signal div_finished : boolean := false;

  -- sine table in RAM
  signal ram : rom_t := (others => (others => '0'));
  signal ram_we : std_logic := '0';
  signal ram_wa, ram_ra : unsigned(7 downto 0) := (others => '0');
  signal ram_wd, ram_rd : q15_t := (others => '0');
  signal ram_done : boolean := false;
begin
  clk <= not clk after 5 ns when not finished else clk;

  u_mac : entity work.mac_unit
    port map (clk => clk, clear => mac_clear, en => mac_en, a => mac_a, b => mac_b,
              acc => mac_acc, result => mac_res, ovf => mac_ovf);

  u_fir : entity work.fir_filter
    port map (clk => clk, coefs => coef_rom, in_valid => fir_valid, in_sample => fir_in,
              out_valid => fir_out_valid, out_sample => fir_out);

  u_cordic : entity work.cordic_rot
    port map (clk => clk, in_valid => cor_valid, angle => cor_angle, out_valid => cor_out_valid,
              cos_o => cor_cos, sin_o => cor_sin);

  u_div : entity work.div_sub
    port map (clk => clk, start => div_start, num => div_num, den => div_den,
              quot => div_q, rem_out => div_r, done => div_done, div0 => div_by0);

  -- Synchronous RAM with one write and one read port.
  ram_proc : process (clk)
  begin
    if rising_edge(clk) then
      if ram_we = '1' then
        ram(to_integer(ram_wa)) <= ram_wd;
      end if;
      ram_rd <= ram(to_integer(ram_ra));
    end if;
  end process;

  -- MAC: dot products of small operands against exact real sums, then a saturation case.
  mac_stim : process
    variable seed : natural := 777;
    variable ref, got : real;
    variable a, b : integer;
    variable low : signed(31 downto 0);
  begin
    wait until rising_edge(clk);
    for t in 1 to n_mac loop
      mac_clear <= '1';
      mac_en <= '0';
      wait until rising_edge(clk);
      mac_clear <= '0';
      ref := 0.0;
      for i in 1 to 16 loop
        seed := next_rand(seed);
        a := (seed mod 4096) - 2048;
        seed := next_rand(seed);
        b := (seed mod 4096) - 2048;
        mac_a <= to_signed(a, 16);
        mac_b <= to_signed(b, 16);
        mac_en <= '1';
        ref := ref + real(a) * real(b);
        wait until rising_edge(clk);
      end loop;
      mac_en <= '0';
      wait until rising_edge(clk);
      wait for 1 ns;
      low := resize(mac_acc, 32);
      got := real(to_integer(low));
      assert got = ref report "mac sum " & real'image(got) & " /= " & real'image(ref) severity failure;
      assert mac_ovf = '0' report "mac overflow flag on small sums" severity failure;
      assert to_integer(mac_res) = integer(floor((ref + 16384.0) / 32768.0))
        report "mac rounded result" severity failure;
    end loop;
    -- Full scale products: the 16 bit result saturates and the overflow flag rises.
    mac_clear <= '1';
    wait until rising_edge(clk);
    mac_clear <= '0';
    mac_a <= to_signed(-32768, 16);
    mac_b <= to_signed(-32768, 16);
    mac_en <= '1';
    for i in 1 to 4 loop
      wait until rising_edge(clk);
    end loop;
    mac_en <= '0';
    wait until rising_edge(clk);
    wait for 1 ns;
    assert mac_res = to_signed(32767, 16) report "mac positive saturation" severity failure;
    assert mac_ovf = '1' report "mac overflow flag" severity failure;
    mac_clear <= '1';
    mac_a <= to_signed(32767, 16);
    mac_b <= to_signed(-32768, 16);
    wait until rising_edge(clk);
    mac_clear <= '0';
    mac_en <= '1';
    for i in 1 to 4 loop
      wait until rising_edge(clk);
    end loop;
    mac_en <= '0';
    wait until rising_edge(clk);
    wait for 1 ns;
    assert mac_res = to_signed(-32768, 16) report "mac negative saturation" severity failure;
    mac_done <= true;
    wait;
  end process;

  -- FIR input: the test signal with a gap every seventh cycle.
  fir_stim : process
    variable n : natural := 0;
    variable k : natural := 0;
  begin
    wait until rising_edge(clk);
    while n < n_fir loop
      k := k + 1;
      if k mod 7 = 0 then
        fir_valid <= '0';
      else
        fir_in <= sample_at(n);
        fir_valid <= '1';
        n := n + 1;
      end if;
      wait until rising_edge(clk);
    end loop;
    fir_valid <= '0';
    wait;
  end process;

  -- FIR check against a real valued convolution of the same quantised input.
  fir_chk : process
    type real_arr_t is array (0 to n_fir - 1) of real;
    variable hist : real_arr_t;
    variable coef_r : real_arr_t;
    variable n : natural := 0;
    variable y, d, worst : real := 0.0;
  begin
    for i in 0 to n_taps - 1 loop
      coef_r(i) := from_q15(coef_rom(i));
    end loop;
    while n < n_fir loop
      wait until rising_edge(clk);
      if fir_out_valid = '1' then
        hist(n) := from_q15(sample_at(n));
        y := 0.0;
        for i in 0 to n_taps - 1 loop
          if n >= i then
            y := y + coef_r(i) * hist(n - i);
          end if;
        end loop;
        d := abs (from_q15(fir_out) - y);
        worst := maximum(worst, d);
        assert d < 4.0 / 32768.0 report "fir sample " & integer'image(n) & " off by " & real'image(d) severity failure;
        n := n + 1;
      end if;
    end loop;
    assert worst > 0.0 report "fir check saw no rounding at all" severity failure;
    fir_done <= true;
    wait;
  end process;

  cordic_stim : process
  begin
    wait until rising_edge(clk);
    for k in 0 to n_angles - 1 loop
      cor_angle <= angle_q(k, n_angles);
      cor_valid <= '1';
      wait until rising_edge(clk);
    end loop;
    cor_valid <= '0';
    wait;
  end process;

  -- CORDIC check: cosine, sine and the length of the vector.
  cordic_chk : process
    variable k : natural := 0;
    variable a, c, s, tol : real;
  begin
    tol := 4.0e-4;
    while k < n_angles loop
      wait until rising_edge(clk);
      if cor_out_valid = '1' then
        a := real(to_integer(angle_q(k, n_angles))) / 65536.0;
        c := real(to_integer(cor_cos)) / 65536.0;
        s := real(to_integer(cor_sin)) / 65536.0;
        assert abs (c - cos(a)) < tol report "cordic cos at " & integer'image(k) severity failure;
        assert abs (s - sin(a)) < tol report "cordic sin at " & integer'image(k) severity failure;
        assert abs (sqrt(c * c + s * s) - 1.0) < tol report "cordic length at " & integer'image(k) severity failure;
        k := k + 1;
      end if;
    end loop;
    cor_done <= true;
    wait;
  end process;

  -- Divider: random operands, a few zero divisors.
  div_proc : process
    variable seed : natural := 31337;
    variable n, d : natural;
  begin
    wait until rising_edge(clk);
    for t in 1 to n_div loop
      seed := next_rand(seed);
      n := seed mod 1024;
      seed := next_rand(seed);
      d := seed mod 64;
      if t mod 25 /= 0 and d = 0 then
        d := 7;
      end if;
      div_num <= to_unsigned(n, 16);
      div_den <= to_unsigned(d, 16);
      div_start <= '1';
      wait until rising_edge(clk);
      div_start <= '0';
      wait until div_done = '1' for 20 us;
      assert div_done = '1' report "divider timed out" severity failure;
      wait until rising_edge(clk);
      if d = 0 then
        assert div_by0 = '1' report "divide by zero not flagged" severity failure;
        assert div_q = x"ffff" report "divide by zero quotient" severity failure;
      else
        assert div_by0 = '0' report "spurious divide by zero" severity failure;
        assert to_integer(div_q) = n / d report "quotient " & integer'image(n) & "/" & integer'image(d) severity failure;
        assert to_integer(div_r) = n mod d report "remainder " & integer'image(n) & "/" & integer'image(d) severity failure;
      end if;
    end loop;
    div_finished <= true;
    wait;
  end process;

  -- Sine table: phase accumulator fills the RAM, then every cell is read back.
  ram_stim : process
    variable phase : unsigned(15 downto 0) := (others => '0');
    variable idx : natural;
    variable e : real;
  begin
    wait until rising_edge(clk);
    for i in 0 to n_ram - 1 loop
      idx := to_integer(phase(15 downto 8));
      ram_wa <= to_unsigned(i, 8);
      ram_wd <= sin_rom(idx);
      ram_we <= '1';
      phase := phase + to_unsigned(256, 16);
      wait until rising_edge(clk);
    end loop;
    ram_we <= '0';
    for i in 0 to n_ram - 1 loop
      ram_ra <= to_unsigned(i, 8);
      wait until rising_edge(clk);
      wait until rising_edge(clk);
      wait for 1 ns;
      e := abs (from_q15(ram_rd) - sin(2.0 * math_pi * real(i) / 256.0));
      assert e < 1.5 / 32768.0 report "sine table at " & integer'image(i) severity failure;
      assert ram_rd = sin_rom(i) report "ram read back at " & integer'image(i) severity failure;
    end loop;
    ram_done <= true;
    wait;
  end process;

  main : process
    variable seed : natural := 99;
    variable v : integer;
    variable x : signed(31 downto 0);
    variable ref : integer;
    variable fl : real;
  begin
    -- Helper functions against integer and real references.
    for k in 0 to 9999 loop
      seed := next_rand(seed);
      v := seed * 31 - 1000000;
      x := to_signed(v, 32);
      ref := v;
      if ref > 32767 then
        ref := 32767;
      elsif ref < -32768 then
        ref := -32768;
      end if;
      assert to_integer(sat(x, 16)) = ref report "sat " & integer'image(v) severity failure;
      for n in 1 to 12 loop
        fl := floor(real(v + 2 ** (n - 1)) / real(2 ** n));
        assert to_integer(round_shift(x, n)) = integer(fl) report "round_shift " & integer'image(v) severity failure;
      end loop;
      assert to_integer(shift_left(to_signed(v mod 4096, 32), 4)) = (v mod 4096) * 16 report "shift_left" severity failure;
      assert to_integer(shift_right(to_signed(-(v mod 4096) - 1, 32), 3)) = (-(v mod 4096) - 1 - 7) / 8
        report "shift_right" severity failure;
    end loop;
    assert to_q15(0.5) = to_signed(16384, 16) report "to_q15 half" severity failure;
    assert to_q15(1.5) = to_signed(32767, 16) report "to_q15 saturates high" severity failure;
    assert to_q15(-1.5) = to_signed(-32768, 16) report "to_q15 saturates low" severity failure;
    assert abs (cordic_gain - 0.607252935) < 1.0e-8 report "cordic gain" severity failure;

    wait until mac_done and fir_done and cor_done and div_finished and ram_done;
    report "fixed-point blocks agree with the reference" severity note;
    report "PASS" severity note;
    finished <= true;
    wait;
  end process;
end architecture tb;
