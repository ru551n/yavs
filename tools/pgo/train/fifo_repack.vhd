-- pgo: top=fifo_repack_tb std=2008 stop=1300us relaxed
-- Byte stream repackers between word widths with sync FIFOs, valid/ready, keep masks and last flags.
-- Four chains with different generics; LFSR idle gaps and backpressure; a scoreboard checks every byte.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package rp_pkg is
  subtype byte_t is std_logic_vector(7 downto 0);

  function bytefn(seed : natural; g : natural) return byte_t;
  function pkt_len(seed : natural; p : natural; maxlen : positive) return positive;
  function total_bytes(seed : natural; n : natural; maxlen : positive) return natural;
  function ones(v : std_logic_vector) return natural;
  function lfsr_next(x : unsigned(15 downto 0)) return unsigned;
end package;

package body rp_pkg is

  -- Expected value of byte number g of a stream.
  function bytefn(seed : natural; g : natural) return byte_t is
  begin
    return std_logic_vector(to_unsigned((g * 167 + (g / 5) * 31 + seed * 59) mod 256, 8));
  end function;

  function pkt_len(seed : natural; p : natural; maxlen : positive) return positive is
  begin
    return 1 + ((p * 7919 + seed * 104729) / 3) mod maxlen;
  end function;

  function total_bytes(seed : natural; n : natural; maxlen : positive) return natural is
    variable t : natural := 0;
  begin
    for p in 0 to n - 1 loop
      t := t + pkt_len(seed, p, maxlen);
    end loop;
    return t;
  end function;

  function ones(v : std_logic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;

  function lfsr_next(x : unsigned(15 downto 0)) return unsigned is
  begin
    return x(14 downto 0) & (x(15) xor x(13) xor x(12) xor x(10));
  end function;

end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

-- Packet source: random packet lengths, random idle gaps, some short mid-packet words.
entity pkt_src is
  generic (
    W      : positive;
    SEED   : natural;
    N_PKTS : natural;
    MAXLEN : positive;
    GAP    : natural range 0 to 3 := 1);
  port (
    clk, rst : in  std_logic;
    m_valid  : out std_logic;
    m_ready  : in  std_logic;
    m_data   : out std_logic_vector(W - 1 downto 0);
    m_keep   : out std_logic_vector(W / 8 - 1 downto 0);
    m_last   : out std_logic);
end entity;

architecture rtl of pkt_src is
  constant B : positive := W / 8;
  signal v   : std_logic := '0';
  signal d   : std_logic_vector(W - 1 downto 0) := (others => '0');
  signal k   : std_logic_vector(B - 1 downto 0) := (others => '0');
  signal l   : std_logic := '0';
begin
  m_valid <= v;
  m_data  <= d;
  m_keep  <= k;
  m_last  <= l;

  process (clk)
    variable p, off, g : natural := 0;
    variable lf        : unsigned(15 downto 0) := to_unsigned(16#1357# + 97 * SEED, 16);
    variable len, n    : natural;
    variable dv        : std_logic_vector(W - 1 downto 0);
    variable kv        : std_logic_vector(B - 1 downto 0);
  begin
    if rising_edge(clk) then
      lf := lfsr_next(lf);
      if rst = '1' then
        p := 0; off := 0; g := 0;
        v <= '0';
      elsif v = '0' or m_ready = '1' then
        if p < N_PKTS and lf(GAP + 2 downto GAP) /= "000" then
          len := pkt_len(SEED, p, MAXLEN);
          n   := minimum(B, len - off);
          if lf(9 downto 8) = "11" and n > 1 then
            n := 1 + to_integer(lf(12 downto 10)) mod n;
          end if;
          dv := (others => '0');
          kv := (others => '0');
          for j in 0 to n - 1 loop
            dv(8 * j + 7 downto 8 * j) := bytefn(SEED, g + j);
            kv(j) := '1';
          end loop;
          v <= '1';
          d <= dv;
          k <= kv;
          if off + n = len then
            l   <= '1';
            off := 0;
            p   := p + 1;
          else
            l   <= '0';
            off := off + n;
          end if;
          g := g + n;
        else
          v <= '0';
        end if;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

-- Width converter. Bytes are appended to a small buffer and drained OUT_W bits at a time;
-- a last flag flushes a partial output word.
entity repack is
  generic (IN_W : positive := 8; OUT_W : positive := 32);
  port (
    clk, rst : in  std_logic;
    s_valid  : in  std_logic;
    s_ready  : out std_logic;
    s_data   : in  std_logic_vector(IN_W - 1 downto 0);
    s_keep   : in  std_logic_vector(IN_W / 8 - 1 downto 0);
    s_last   : in  std_logic;
    m_valid  : out std_logic;
    m_ready  : in  std_logic;
    m_data   : out std_logic_vector(OUT_W - 1 downto 0);
    m_keep   : out std_logic_vector(OUT_W / 8 - 1 downto 0);
    m_last   : out std_logic);
end entity;

architecture rtl of repack is
  constant IB  : positive := IN_W / 8;
  constant OB  : positive := OUT_W / 8;
  constant CAP : positive := IB + OB;
  type buf_t is array (0 to CAP - 1) of byte_t;
  signal buf    : buf_t := (others => (others => '0'));
  signal cnt    : natural range 0 to CAP := 0;
  signal last_p : std_logic := '0';
  signal rdy    : std_logic;
  signal ov, ol : std_logic := '0';
  signal od     : std_logic_vector(OUT_W - 1 downto 0) := (others => '0');
  signal ok     : std_logic_vector(OB - 1 downto 0) := (others => '0');
begin
  rdy     <= '1' when cnt <= CAP - IB and last_p = '0' and rst = '0' else '0';
  s_ready <= rdy;
  m_valid <= ov;
  m_data  <= od;
  m_keep  <= ok;
  m_last  <= ol;

  process (clk)
    variable b    : buf_t;
    variable n, k : natural range 0 to CAP;
    variable lp   : std_logic;
    variable ov_v : std_logic_vector(OUT_W - 1 downto 0);
    variable ok_v : std_logic_vector(OB - 1 downto 0);
    variable ol_v : std_logic;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        cnt <= 0; last_p <= '0'; ov <= '0';
      else
        b := buf; n := cnt; lp := last_p;
        if ov = '0' or m_ready = '1' then
          if n >= OB or (lp = '1' and n > 0) then
            k := minimum(n, OB);
            for j in 0 to OB - 1 loop
              if j < k then
                ov_v(8 * j + 7 downto 8 * j) := b(j);
                ok_v(j) := '1';
              else
                ov_v(8 * j + 7 downto 8 * j) := (others => '0');
                ok_v(j) := '0';
              end if;
            end loop;
            for j in 0 to CAP - 1 loop
              if j + k < CAP then
                b(j) := b(j + k);
              else
                b(j) := (others => '0');
              end if;
            end loop;
            n    := n - k;
            ol_v := '0';
            if n = 0 then
              ol_v := lp;
              lp   := '0';
            end if;
            od <= ov_v; ok <= ok_v; ol <= ol_v; ov <= '1';
          else
            ov <= '0';
          end if;
        end if;
        if s_valid = '1' and rdy = '1' then
          for j in 0 to IB - 1 loop
            if s_keep(j) = '1' then
              b(n) := s_data(8 * j + 7 downto 8 * j);
              n    := n + 1;
            end if;
          end loop;
          if s_last = '1' then
            lp := '1';
          end if;
        end if;
        buf <= b; cnt <= n; last_p <= lp;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Show-ahead FIFO, power-of-two depth, wrap bit on the pointers. Data, keep and last share one memory.
entity sync_fifo is
  generic (W : positive := 8; AW : positive := 2);
  port (
    clk, rst : in  std_logic;
    s_valid  : in  std_logic;
    s_ready  : out std_logic;
    s_data   : in  std_logic_vector(W - 1 downto 0);
    s_keep   : in  std_logic_vector(W / 8 - 1 downto 0);
    s_last   : in  std_logic;
    m_valid  : out std_logic;
    m_ready  : in  std_logic;
    m_data   : out std_logic_vector(W - 1 downto 0);
    m_keep   : out std_logic_vector(W / 8 - 1 downto 0);
    m_last   : out std_logic;
    level    : out natural);
end entity;

architecture rtl of sync_fifo is
  constant DEPTH : positive := 2 ** AW;
  subtype word_t is std_logic_vector(W + W / 8 downto 0);
  type mem_t is array (0 to DEPTH - 1) of word_t;
  signal mem         : mem_t;
  signal wp, rp      : unsigned(AW downto 0) := (others => '0');
  signal full, empty : std_logic;
  signal head        : word_t;
begin
  full    <= '1' when wp(AW) /= rp(AW) and wp(AW - 1 downto 0) = rp(AW - 1 downto 0) else '0';
  empty   <= '1' when wp = rp else '0';
  s_ready <= not full;
  m_valid <= not empty;
  head    <= mem(to_integer(rp(AW - 1 downto 0)));
  m_data  <= head(W - 1 downto 0);
  m_keep  <= head(W + W / 8 - 1 downto W);
  m_last  <= head(W + W / 8);
  level   <= to_integer(wp - rp);

  wr : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        wp <= (others => '0');
      elsif s_valid = '1' and full = '0' then
        mem(to_integer(wp(AW - 1 downto 0))) <= s_last & s_keep & s_data;
        wp <= wp + 1;
      end if;
    end if;
  end process;

  rd : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        rp <= (others => '0');
      elsif m_ready = '1' and empty = '0' then
        rp <= rp + 1;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

-- Packet sink with random backpressure; checks every byte, keep shape and packet length.
entity pkt_sink is
  generic (
    W        : positive;
    SEED     : natural;
    N_PKTS   : natural;
    MAXLEN   : positive;
    RDY_BITS : natural range 1 to 3 := 1);
  port (
    clk, rst : in  std_logic;
    s_valid  : in  std_logic;
    s_ready  : out std_logic;
    s_data   : in  std_logic_vector(W - 1 downto 0);
    s_keep   : in  std_logic_vector(W / 8 - 1 downto 0);
    s_last   : in  std_logic;
    done     : out std_logic;
    nbytes   : out natural);
end entity;

architecture rtl of pkt_sink is
  signal rdy : std_logic := '0';
begin
  s_ready <= rdy;

  gate : process (clk)
    variable lf : unsigned(15 downto 0) := to_unsigned(16#2468# + 131 * SEED, 16);
  begin
    if rising_edge(clk) then
      lf  := lfsr_next(lf);
      rdy <= or lf(RDY_BITS + 3 downto 4);
    end if;
  end process;

  check : process (clk)
    variable p, off, g : natural := 0;
    variable len       : positive;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        p := 0; off := 0; g := 0;
        done <= '0';
      elsif s_valid = '1' and rdy = '1' then
        assert p < N_PKTS report "sink " & integer'image(SEED) & ": data after last packet" severity failure;
        len := pkt_len(SEED, p, MAXLEN);
        for j in 0 to s_keep'length - 1 loop
          if s_keep(j) = '1' then
            assert j = 0 or s_keep(j - 1) = '1'
              report "sink " & integer'image(SEED) & ": keep not contiguous" severity failure;
            assert s_data(8 * j + 7 downto 8 * j) = bytefn(SEED, g)
              report "sink " & integer'image(SEED) & ": byte " & integer'image(g) & " packet "
                     & integer'image(p) & " got " & to_hstring(s_data(8 * j + 7 downto 8 * j))
                     & " expected " & to_hstring(bytefn(SEED, g)) severity failure;
            g   := g + 1;
            off := off + 1;
          end if;
        end loop;
        assert off <= len report "sink " & integer'image(SEED) & ": packet " & integer'image(p) & " too long" severity failure;
        if s_last = '1' then
          assert off = len
            report "sink " & integer'image(SEED) & ": packet " & integer'image(p) & " length " & integer'image(off)
                   & " expected " & integer'image(len) severity failure;
          off := 0;
          p   := p + 1;
        else
          assert s_keep = (s_keep'range => '1')
            report "sink " & integer'image(SEED) & ": short word inside packet" severity failure;
        end if;
        nbytes <= g;
        if p = N_PKTS then
          done <= '1';
        end if;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

-- Source, three repackers, up to two FIFOs, sink.
entity repack_chain is
  generic (
    W0, W1, W2, W3 : positive;
    AW1, AW2       : positive := 1;
    F2             : boolean  := true;
    SEED           : natural;
    N_PKTS         : natural;
    MAXLEN         : positive;
    GAP            : natural range 0 to 3 := 1;
    RDY_BITS       : natural range 1 to 3 := 1);
  port (
    clk, rst  : in  std_logic;
    done      : out std_logic;
    bytes_in  : out natural;
    bytes_out : out natural);
end entity;

architecture rtl of repack_chain is
  type bus_t is record
    valid, ready, last : std_logic;
  end record;

  signal a_b, b_b, c_b, d_b, e_b, f_b : bus_t;
  signal a_d : std_logic_vector(W0 - 1 downto 0);
  signal a_k : std_logic_vector(W0 / 8 - 1 downto 0);
  signal b_d : std_logic_vector(W1 - 1 downto 0);
  signal b_k : std_logic_vector(W1 / 8 - 1 downto 0);
  signal c_d : std_logic_vector(W1 - 1 downto 0);
  signal c_k : std_logic_vector(W1 / 8 - 1 downto 0);
  signal d_d : std_logic_vector(W2 - 1 downto 0);
  signal d_k : std_logic_vector(W2 / 8 - 1 downto 0);
  signal e_d : std_logic_vector(W2 - 1 downto 0);
  signal e_k : std_logic_vector(W2 / 8 - 1 downto 0);
  signal f_d : std_logic_vector(W3 - 1 downto 0);
  signal f_k : std_logic_vector(W3 / 8 - 1 downto 0);
  signal lvl1, lvl2 : natural := 0;
  signal hwm1, hwm2 : natural := 0;
begin
  src : entity work.pkt_src
    generic map (W => W0, SEED => SEED, N_PKTS => N_PKTS, MAXLEN => MAXLEN, GAP => GAP)
    port map (clk => clk, rst => rst, m_valid => a_b.valid, m_ready => a_b.ready,
              m_data => a_d, m_keep => a_k, m_last => a_b.last);

  rp1 : entity work.repack
    generic map (IN_W => W0, OUT_W => W1)
    port map (clk => clk, rst => rst,
              s_valid => a_b.valid, s_ready => a_b.ready, s_data => a_d, s_keep => a_k, s_last => a_b.last,
              m_valid => b_b.valid, m_ready => b_b.ready, m_data => b_d, m_keep => b_k, m_last => b_b.last);

  fifo1 : entity work.sync_fifo
    generic map (W => W1, AW => AW1)
    port map (clk => clk, rst => rst,
              s_valid => b_b.valid, s_ready => b_b.ready, s_data => b_d, s_keep => b_k, s_last => b_b.last,
              m_valid => c_b.valid, m_ready => c_b.ready, m_data => c_d, m_keep => c_k, m_last => c_b.last,
              level => lvl1);

  rp2 : entity work.repack
    generic map (IN_W => W1, OUT_W => W2)
    port map (clk => clk, rst => rst,
              s_valid => c_b.valid, s_ready => c_b.ready, s_data => c_d, s_keep => c_k, s_last => c_b.last,
              m_valid => d_b.valid, m_ready => d_b.ready, m_data => d_d, m_keep => d_k, m_last => d_b.last);

  with_f2 : if F2 generate
    fifo2 : entity work.sync_fifo
      generic map (W => W2, AW => AW2)
      port map (clk => clk, rst => rst,
                s_valid => d_b.valid, s_ready => d_b.ready, s_data => d_d, s_keep => d_k, s_last => d_b.last,
                m_valid => e_b.valid, m_ready => e_b.ready, m_data => e_d, m_keep => e_k, m_last => e_b.last,
                level => lvl2);
  else generate
    e_b.valid <= d_b.valid;
    d_b.ready <= e_b.ready;
    e_b.last  <= d_b.last;
    e_d       <= d_d;
    e_k       <= d_k;
    lvl2      <= 0;
  end generate;

  rp3 : entity work.repack
    generic map (IN_W => W2, OUT_W => W3)
    port map (clk => clk, rst => rst,
              s_valid => e_b.valid, s_ready => e_b.ready, s_data => e_d, s_keep => e_k, s_last => e_b.last,
              m_valid => f_b.valid, m_ready => f_b.ready, m_data => f_d, m_keep => f_k, m_last => f_b.last);

  sink : entity work.pkt_sink
    generic map (W => W3, SEED => SEED, N_PKTS => N_PKTS, MAXLEN => MAXLEN, RDY_BITS => RDY_BITS)
    port map (clk => clk, rst => rst,
              s_valid => f_b.valid, s_ready => f_b.ready, s_data => f_d, s_keep => f_k, s_last => f_b.last,
              done => done, nbytes => bytes_out);

  -- Bytes entering the chain.
  count_in : process (clk)
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        n := 0;
      elsif a_b.valid = '1' and a_b.ready = '1' then
        n := n + ones(a_k);
      end if;
      bytes_in <= n;
    end if;
  end process;

  -- FIFO high-water marks must stay inside the depth.
  watch : process (clk)
  begin
    if rising_edge(clk) then
      if lvl1 > hwm1 then hwm1 <= lvl1; end if;
      if lvl2 > hwm2 then hwm2 <= lvl2; end if;
      assert hwm1 <= 2 ** AW1 report "fifo1 overfull" severity failure;
      assert hwm2 <= 2 ** AW2 report "fifo2 overfull" severity failure;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity fifo_repack_tb is
end entity;

architecture sim of fifo_repack_tb is
  constant N_PKTS : natural := 3000;
  constant PERIOD : time := 10 ns;

  signal clk  : std_logic := '0';
  signal rst  : std_logic := '1';
  signal fin  : boolean   := false;
  signal done : std_logic_vector(0 to 3);
  type nat_arr_t is array (0 to 3) of natural;
  signal bin, bout : nat_arr_t;
begin
  clk <= not clk after PERIOD / 2 when not fin;
  rst <= '0' after 5 * PERIOD;

  chain_a : entity work.repack_chain
    generic map (W0 => 8, W1 => 32, W2 => 24, W3 => 8, AW1 => 2, AW2 => 1, SEED => 1,
                 N_PKTS => N_PKTS, MAXLEN => 37, GAP => 1, RDY_BITS => 1)
    port map (clk => clk, rst => rst, done => done(0), bytes_in => bin(0), bytes_out => bout(0));

  chain_b : entity work.repack_chain
    generic map (W0 => 16, W1 => 64, W2 => 40, W3 => 16, AW1 => 3, AW2 => 2, SEED => 2,
                 N_PKTS => N_PKTS, MAXLEN => 61, GAP => 2, RDY_BITS => 2)
    port map (clk => clk, rst => rst, done => done(1), bytes_in => bin(1), bytes_out => bout(1));

  chain_c : entity work.repack_chain
    generic map (W0 => 24, W1 => 24, W2 => 8, W3 => 32, F2 => false, SEED => 3,
                 N_PKTS => N_PKTS, MAXLEN => 19, GAP => 3, RDY_BITS => 1)
    port map (clk => clk, rst => rst, done => done(2), bytes_in => bin(2), bytes_out => bout(2));

  chain_d : entity work.repack_chain
    generic map (W0 => 32, W1 => 8, W2 => 16, W3 => 8, AW1 => 3, AW2 => 2, SEED => 4,
                 N_PKTS => N_PKTS, MAXLEN => 9, GAP => 0, RDY_BITS => 3)
    port map (clk => clk, rst => rst, done => done(3), bytes_in => bin(3), bytes_out => bout(3));

  main : process
    type len_arr_t is array (0 to 3) of positive;
    constant MAXL : len_arr_t := (37, 61, 19, 9);
  begin
    wait until and done = '1';
    wait for 10 * PERIOD;
    for i in 0 to 3 loop
      assert bin(i) = total_bytes(i + 1, N_PKTS, MAXL(i))
        report "chain " & integer'image(i) & " bytes in " & integer'image(bin(i)) severity failure;
      assert bout(i) = bin(i)
        report "chain " & integer'image(i) & " bytes out " & integer'image(bout(i)) severity failure;
    end loop;
    fin <= true;
    report "PASS" severity note;
    wait;
  end process;
end architecture;
