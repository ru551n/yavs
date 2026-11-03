-- pgo: top=wide_buf_tb std=2008 stop=650us
-- Buffer writer data path: eight 64-bit streams merged into a 512-bit word, shifted/aligned/masked
-- by queued commands, then split into a response with parity, popcount and flags. Wide slice work.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package wide_pkg is
  constant LANES : natural := 8;
  constant LW    : natural := 64;
  constant BW    : natural := LANES * LW;

  subtype lane_t is std_logic_vector(LW - 1 downto 0);
  subtype wide_t is std_logic_vector(BW - 1 downto 0);
  subtype be_t is std_logic_vector(BW / 8 - 1 downto 0);
  type lane_arr_t is array (0 to LANES - 1) of lane_t;
  type quad_arr_t is array (0 to 3) of std_logic_vector(BW / 4 - 1 downto 0);

  type cmd_t is record
    op  : std_logic_vector(2 downto 0);
    sh  : unsigned(5 downto 0);
    tag : unsigned(15 downto 0);
    be  : be_t;
  end record;

  type rsp_t is record
    seq   : unsigned(31 downto 0);
    tag   : unsigned(15 downto 0);
    data  : wide_t;
    lanes : lane_arr_t;
    quads : quad_arr_t;
    par   : std_logic_vector(LANES - 1 downto 0);
    pop   : unsigned(9 downto 0);
    flags : std_logic_vector(3 downto 0);
  end record;

  function mix(i : natural; n : natural) return lane_t;
  function make_cmd(n : natural) return cmd_t;
  function parity(v : std_logic_vector) return std_logic;
  function popcount(v : std_logic_vector) return natural;
  function bswap(v : std_logic_vector) return std_logic_vector;
  function rotl(v : std_logic_vector; n : natural) return std_logic_vector;
  function merge_lanes(l : lane_arr_t) return wide_t;
  function apply_op(c : cmd_t; w : wide_t; prev : wide_t) return wide_t;
  function make_rsp(seq : unsigned; tag : unsigned; w : wide_t; prev : wide_t) return rsp_t;
end package;

package body wide_pkg is

  -- Deterministic 64-bit word for stream i, sequence number n.
  function mix(i : natural; n : natural) return lane_t is
    variable x : unsigned(63 downto 0);
  begin
    x := shift_left(to_unsigned(n, 64), 17) + to_unsigned(i * 40503 + 12345, 64);
    x := x xor shift_right(x, 29);
    x := resize(x * to_unsigned(16#5851F42D#, 64), 64);
    x := x xor shift_right(x, 31);
    x := x + to_unsigned(n, 64);
    return std_logic_vector(x);
  end function;

  function make_cmd(n : natural) return cmd_t is
    variable a, b : lane_t;
    variable c    : cmd_t;
  begin
    a     := mix(8, n);
    b     := mix(9, n);
    c.op  := a(2 downto 0);
    c.sh  := unsigned(a(13 downto 8));
    c.tag := unsigned(a(31 downto 16));
    c.be  := b;
    return c;
  end function;

  function parity(v : std_logic_vector) return std_logic is
    variable r : std_logic := '0';
  begin
    for i in v'range loop
      r := r xor v(i);
    end loop;
    return r;
  end function;

  function popcount(v : std_logic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function bswap(v : std_logic_vector) return std_logic_vector is
    constant nb : natural := v'length / 8;
    variable x  : std_logic_vector(v'length - 1 downto 0) := v;
    variable r  : std_logic_vector(v'length - 1 downto 0);
  begin
    for j in 0 to nb - 1 loop
      r(8 * j + 7 downto 8 * j) := x(8 * (nb - 1 - j) + 7 downto 8 * (nb - 1 - j));
    end loop;
    return r;
  end function;

  function rotl(v : std_logic_vector; n : natural) return std_logic_vector is
    constant len : natural := v'length;
    constant k   : natural := n mod len;
    variable x   : std_logic_vector(len - 1 downto 0) := v;
  begin
    if k = 0 then
      return x;
    end if;
    return x(len - 1 - k downto 0) & x(len - 1 downto len - k);
  end function;

  function merge_lanes(l : lane_arr_t) return wide_t is
    variable o : wide_t;
  begin
    for i in 0 to LANES - 1 loop
      o(i * LW + LW - 1 downto i * LW) := l(i);
    end loop;
    return o;
  end function;

  function apply_op(c : cmd_t; w : wide_t; prev : wide_t) return wide_t is
    constant sb : natural := to_integer(c.sh) * 8;
    variable r  : wide_t;
  begin
    case c.op is
      when "000" =>
        r := w;
      when "001" =>
        r := w sll sb;
      when "010" =>
        r := w srl sb;
      when "011" =>
        if sb = 0 then
          r := w;
        else
          r := w(BW - 1 - sb downto 0) & w(BW - 1 downto BW - sb);
        end if;
      when "100" =>
        if sb = 0 then
          r := w;
        else
          r := w(BW - 1 - sb downto 0) & prev(BW - 1 downto BW - sb);
        end if;
      when "101" =>
        for j in 0 to BW / 8 - 1 loop
          if c.be(j) = '1' then
            r(8 * j + 7 downto 8 * j) := w(8 * j + 7 downto 8 * j);
          else
            r(8 * j + 7 downto 8 * j) := prev(8 * j + 7 downto 8 * j);
          end if;
        end loop;
      when "110" =>
        r := (w and prev) or (not w and (prev xor rotl(w, 13)));
      when others =>
        r := bswap(w(BW / 2 - 1 downto 0)) & bswap(w(BW - 1 downto BW / 2));
    end case;
    return r;
  end function;

  function make_rsp(seq : unsigned; tag : unsigned; w : wide_t; prev : wide_t) return rsp_t is
    alias lo_h : std_logic_vector(BW / 2 - 1 downto 0) is w(BW / 2 - 1 downto 0);
    alias hi_h : std_logic_vector(BW / 2 - 1 downto 0) is w(BW - 1 downto BW / 2);
    variable r : rsp_t;
  begin
    r.seq  := seq;
    r.tag  := tag;
    r.data := w;
    for i in 0 to LANES - 1 loop
      r.lanes(i) := w(i * LW + LW - 1 downto i * LW);
    end loop;
    for q in r.quads'range loop
      r.quads(q) := w((q + 1) * (BW / 4) - 1 downto q * (BW / 4));
    end loop;
    r.par       := (others => '0');
    r.pop       := to_unsigned(popcount(w), r.pop'length);
    r.flags(0)  := '1' when w = prev else '0';
    r.flags(1)  := '1' when unsigned(hi_h) > unsigned(lo_h) else '0';
    r.flags(2)  := '1' when w = (w'range => '0') else '0';
    r.flags(3)  := '1' when (w and prev) = prev else '0';
    return r;
  end function;

end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.wide_pkg.all;

-- Synchronous command FIFO, show-ahead read, almost-full flag.
entity cmd_fifo is
  generic (DEPTH : positive := 8);
  port (
    clk, rst : in  std_logic;
    push     : in  std_logic;
    din      : in  cmd_t;
    full     : out std_logic;
    pop      : in  std_logic;
    dout     : out cmd_t;
    valid    : out std_logic);
end entity;

architecture rtl of cmd_fifo is
  type mem_t is array (0 to DEPTH - 1) of cmd_t;
  signal mem   : mem_t;
  signal wp    : integer range 0 to DEPTH - 1 := 0;
  signal rp    : integer range 0 to DEPTH - 1 := 0;
  signal count : integer range 0 to DEPTH     := 0;
begin
  process (clk)
    variable c : integer range 0 to DEPTH;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        wp <= 0; rp <= 0; count <= 0;
      else
        c := count;
        if push = '1' then
          mem(wp) <= din;
          wp      <= (wp + 1) mod DEPTH;
          c       := c + 1;
        end if;
        if pop = '1' then
          rp <= (rp + 1) mod DEPTH;
          c  := c - 1;
        end if;
        count <= c;
      end if;
    end if;
  end process;

  dout  <= mem(rp);
  valid <= '1' when count > 0 else '0';
  full  <= '1' when count >= DEPTH - 2 else '0';
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.wide_pkg.all;

-- Same FIFO shape for the response record.
entity rsp_fifo is
  generic (DEPTH : positive := 8);
  port (
    clk, rst : in  std_logic;
    push     : in  std_logic;
    din      : in  rsp_t;
    full     : out std_logic;
    pop      : in  std_logic;
    dout     : out rsp_t;
    valid    : out std_logic);
end entity;

architecture rtl of rsp_fifo is
  type mem_t is array (0 to DEPTH - 1) of rsp_t;
  signal mem   : mem_t;
  signal wp    : natural range 0 to DEPTH - 1 := 0;
  signal rp    : natural range 0 to DEPTH - 1 := 0;
  signal count : natural range 0 to DEPTH     := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        wp <= 0; rp <= 0; count <= 0;
      else
        if push = '1' then
          mem(wp) <= din;
          wp      <= (wp + 1) mod DEPTH;
        end if;
        if pop = '1' then
          rp <= (rp + 1) mod DEPTH;
        end if;
        if push = '1' and pop = '0' then
          count <= count + 1;
        elsif push = '0' and pop = '1' then
          count <= count - 1;
        end if;
      end if;
    end if;
  end process;

  dout  <= mem(rp);
  valid <= '1' when count > 0 else '0';
  full  <= '1' when count >= DEPTH - 2 else '0';
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.wide_pkg.all;

-- Three stages: merge lanes, apply command, build response. Stalls when the response FIFO fills.
entity buf_writer is
  port (
    clk, rst   : in  std_logic;
    lane_valid : in  std_logic_vector(LANES - 1 downto 0);
    lane_data  : in  lane_arr_t;
    lane_take  : out std_logic;
    cmd_valid  : in  std_logic;
    cmd_data   : in  cmd_t;
    cmd_pop    : out std_logic;
    rsp_full   : in  std_logic;
    rsp_push   : out std_logic;
    rsp_data   : out rsp_t);
end entity;

architecture rtl of buf_writer is
  signal v1, v2      : std_logic := '0';
  signal c1          : cmd_t;
  signal w1, w2, wp  : wide_t := (others => '0');
  signal seq_cnt     : unsigned(31 downto 0) := (others => '0');
  signal seq1, seq2  : unsigned(31 downto 0) := (others => '0');
  signal tag2        : unsigned(15 downto 0) := (others => '0');
  signal all_valid   : std_logic;
  signal adv, take   : std_logic;
  signal par_s       : std_logic_vector(LANES - 1 downto 0);
  signal rsp_c       : rsp_t;
  alias  be_lo       : std_logic_vector(7 downto 0) is c1.be(7 downto 0);
begin
  all_valid <= and lane_valid;
  adv       <= not rsp_full;
  take      <= all_valid and cmd_valid and adv;
  lane_take <= take;
  cmd_pop   <= take;

  stage_a : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        v1      <= '0';
        seq_cnt <= (others => '0');
      elsif adv = '1' then
        v1 <= take;
        if take = '1' then
          w1      <= merge_lanes(lane_data);
          c1      <= cmd_data;
          seq1    <= seq_cnt;
          seq_cnt <= seq_cnt + 1;
        end if;
      end if;
    end if;
  end process;

  stage_b : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        v2 <= '0';
      elsif adv = '1' then
        v2 <= v1;
        if v1 = '1' then
          w2   <= apply_op(c1, w1, w2);
          wp   <= w2;
          seq2 <= seq1;
          tag2 <= c1.tag;
        end if;
      end if;
    end if;
  end process;

  par_gen : for i in 0 to LANES - 1 generate
    par_s(i) <= parity(w2(i * LW + LW - 1 downto i * LW));
  end generate;

  stage_c : process (all)
    variable r : rsp_t;
  begin
    r     := make_rsp(seq2, tag2, w2, wp);
    r.par := par_s;
    rsp_c <= r;
  end process;

  rsp_data <= rsp_c;
  rsp_push <= v2 and adv;

  -- Sanity: byte-enable alias follows the staged command.
  assert be_lo'length = 8 report "alias width" severity failure;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.wide_pkg.all;

entity wide_buf_tb is
end entity;

architecture sim of wide_buf_tb is
  constant N_WORDS : natural := 40000;
  constant PERIOD  : time    := 10 ns;

  type byte_tab_t is array (0 to 255) of natural;

  function init_tab return byte_tab_t is
    variable t : byte_tab_t;
    variable n : natural;
  begin
    for v in 0 to 255 loop
      n := 0;
      for b in 0 to 7 loop
        if ((v / 2 ** b) mod 2) = 1 then
          n := n + 1;
        end if;
      end loop;
      t(v) := n;
    end loop;
    return t;
  end function;

  constant BYTE_POP : byte_tab_t := init_tab;

  -- Reference model: bit-by-bit loops with integer indexes.
  function m_merge(n : natural) return wide_t is
    variable r : wide_t;
    variable l : lane_t;
  begin
    for i in 0 to LANES - 1 loop
      l := mix(i, n);
      for b in 0 to LW - 1 loop
        r(i * LW + b) := l(b);
      end loop;
    end loop;
    return r;
  end function;

  function m_op(c : cmd_t; w : wide_t; prev : wide_t) return wide_t is
    variable r   : wide_t := (others => '0');
    variable rot : wide_t;
    constant k   : natural := to_integer(c.sh) * 8;
  begin
    case to_integer(unsigned(c.op)) is
      when 0 => r := w;
      when 1 =>
        for b in 0 to BW - 1 loop
          if b >= k then r(b) := w(b - k); end if;
        end loop;
      when 2 =>
        for b in 0 to BW - 1 loop
          if b + k < BW then r(b) := w(b + k); end if;
        end loop;
      when 3 =>
        for b in 0 to BW - 1 loop
          r((b + k) mod BW) := w(b);
        end loop;
      when 4 =>
        for b in 0 to BW - 1 loop
          if b >= k then r(b) := w(b - k); else r(b) := prev(BW - k + b); end if;
        end loop;
      when 5 =>
        for b in 0 to BW - 1 loop
          if c.be(b / 8) = '1' then r(b) := w(b); else r(b) := prev(b); end if;
        end loop;
      when 6 =>
        for b in 0 to BW - 1 loop
          rot((b + 13) mod BW) := w(b);
        end loop;
        for b in 0 to BW - 1 loop
          r(b) := (w(b) and prev(b)) or ((not w(b)) and (prev(b) xor rot(b)));
        end loop;
      when others =>
        for j in 0 to BW / 8 - 1 loop
          for t in 0 to 7 loop
            r(8 * j + t) := w(8 * (BW / 8 - 1 - j) + t);
          end loop;
        end loop;
    end case;
    return r;
  end function;

  function m_pop(w : wide_t) return natural is
    variable n : natural := 0;
  begin
    for j in 0 to BW / 8 - 1 loop
      n := n + BYTE_POP(to_integer(unsigned(w(8 * j + 7 downto 8 * j))));
    end loop;
    return n;
  end function;

  function m_par(w : wide_t; i : natural) return std_logic is
    variable p : natural := 0;
  begin
    for b in 0 to LW - 1 loop
      if w(i * LW + b) = '1' then p := p + 1; end if;
    end loop;
    if p mod 2 = 1 then return '1'; else return '0'; end if;
  end function;

  function m_hi_gt_lo(w : wide_t) return boolean is
  begin
    for b in BW / 2 - 1 downto 0 loop
      if w(b + BW / 2) /= w(b) then
        return w(b + BW / 2) = '1';
      end if;
    end loop;
    return false;
  end function;

  -- True when every bit set in p is also set in w.
  function m_covers(w : wide_t; p : wide_t) return boolean is
  begin
    for b in 0 to BW - 1 loop
      if p(b) = '1' and w(b) /= '1' then return false; end if;
    end loop;
    return true;
  end function;

  signal clk, rst   : std_logic := '0';
  signal done       : boolean := false;
  signal lane_valid : std_logic_vector(LANES - 1 downto 0) := (others => '0');
  signal lane_data  : lane_arr_t;
  signal lane_take  : std_logic;
  signal cmd_push, cmd_full, cmd_pop, cmd_valid : std_logic := '0';
  signal cmd_in, cmd_out : cmd_t;
  signal rsp_push, rsp_full, rsp_pop, rsp_valid : std_logic;
  signal rsp_in, rsp_out : rsp_t;
  signal rdy        : std_logic := '0';
  signal checked    : natural := 0;
begin
  clk <= not clk after PERIOD / 2 when not done;
  rst <= '1', '0' after 4 * PERIOD;

  lanes_gen : for i in 0 to LANES - 1 generate
  begin
    src : process (clk)
      variable lf   : unsigned(15 downto 0) := to_unsigned(16#ACE1# + 37 * i, 16);
      variable n    : natural := 0;
      variable gate : std_logic;
    begin
      if rising_edge(clk) then
        lf   := lf(14 downto 0) & (lf(15) xor lf(13) xor lf(12) xor lf(10));
        gate := lf(0) or lf(3) or lf(6) or lf(9);
        if rst = '1' then
          n := 0;
          lane_valid(i) <= '0';
          lane_data(i)  <= mix(i, 0);
        elsif lane_take = '1' then
          n := n + 1;
          lane_data(i)  <= mix(i, n);
          lane_valid(i) <= gate;
        else
          lane_valid(i) <= lane_valid(i) or gate;
        end if;
      end if;
    end process;
  end generate;

  cmd_prod : process (clk)
    variable lf : unsigned(15 downto 0) := x"B4D1";
    variable k  : natural := 0;
  begin
    if rising_edge(clk) then
      lf       := lf(14 downto 0) & (lf(15) xor lf(13) xor lf(12) xor lf(10));
      cmd_push <= '0';
      if rst = '0' and cmd_full = '0' and k < N_WORDS and lf(2 downto 0) /= "111" then
        cmd_push <= '1';
        cmd_in   <= make_cmd(k);
        k        := k + 1;
      end if;
    end if;
  end process;

  cmd_q : entity work.cmd_fifo
    generic map (DEPTH => 8)
    port map (clk => clk, rst => rst, push => cmd_push, din => cmd_in, full => cmd_full,
              pop => cmd_pop, dout => cmd_out, valid => cmd_valid);

  dut : entity work.buf_writer
    port map (clk => clk, rst => rst, lane_valid => lane_valid, lane_data => lane_data,
              lane_take => lane_take, cmd_valid => cmd_valid, cmd_data => cmd_out,
              cmd_pop => cmd_pop, rsp_full => rsp_full, rsp_push => rsp_push, rsp_data => rsp_in);

  rsp_q : entity work.rsp_fifo
    generic map (DEPTH => 16)
    port map (clk => clk, rst => rst, push => rsp_push, din => rsp_in, full => rsp_full,
              pop => rsp_pop, dout => rsp_out, valid => rsp_valid);

  rdy_gen : process (clk)
    variable lf : unsigned(15 downto 0) := x"1D2B";
  begin
    if rising_edge(clk) then
      lf  := lf(14 downto 0) & (lf(15) xor lf(13) xor lf(12) xor lf(10));
      rdy <= lf(1) or lf(4) or lf(7);
    end if;
  end process;

  rsp_pop <= rsp_valid and rdy;

  check : process (clk)
    variable n    : natural := 0;
    variable prev : wide_t := (others => '0');
    variable w, o : wide_t;
    variable c    : cmd_t;
    variable r    : rsp_t;
  begin
    if rising_edge(clk) then
      if rst = '0' and rsp_pop = '1' then
        r := rsp_out;
        c := make_cmd(n);
        w := m_merge(n);
        o := m_op(c, w, prev);
        assert to_integer(r.seq) = n
          report "seq " & integer'image(to_integer(r.seq)) & " expected " & integer'image(n) severity failure;
        assert r.tag = c.tag report "tag at word " & integer'image(n) severity failure;
        assert r.data = o
          report "data at word " & integer'image(n) & " op " & integer'image(to_integer(unsigned(c.op)))
                 & " got " & to_hstring(r.data) & " exp " & to_hstring(o) severity failure;
        for i in 0 to LANES - 1 loop
          assert r.lanes(i) = o(i * LW + LW - 1 downto i * LW)
            report "lane " & integer'image(i) & " at word " & integer'image(n) severity failure;
          assert r.par(i) = m_par(o, i) report "parity at word " & integer'image(n) severity failure;
        end loop;
        for q in 0 to 3 loop
          assert r.quads(q) = o(q * 128 + 127 downto q * 128)
            report "quad at word " & integer'image(n) severity failure;
        end loop;
        assert to_integer(r.pop) = m_pop(o) report "popcount at word " & integer'image(n) severity failure;
        assert (r.flags(0) = '1') = (o = prev) report "flag eq at word " & integer'image(n) severity failure;
        assert (r.flags(1) = '1') = m_hi_gt_lo(o) report "flag gt at word " & integer'image(n) severity failure;
        assert (r.flags(2) = '1') = (m_pop(o) = 0) report "flag zero at word " & integer'image(n) severity failure;
        assert (r.flags(3) = '1') = m_covers(o, prev)
          report "flag sub at word " & integer'image(n) severity failure;
        prev := o;
        n    := n + 1;
        checked <= n;
      end if;
    end if;
  end process;

  main : process
  begin
    wait until checked = N_WORDS;
    wait for 5 * PERIOD;
    assert checked = N_WORDS report "extra words" severity failure;
    assert cmd_valid = '0' report "commands left" severity failure;
    done <= true;
    report "PASS" severity note;
    wait;
  end process;
end architecture;
