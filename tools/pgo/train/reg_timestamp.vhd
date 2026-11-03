-- pgo: top=reg_timestamp_tb std=2008 stop=5ms relaxed
-- Bus-attached register block: address decode, 64-bit timestamp counter,
-- capture/compare, event FSM, shadow registers and two-clock synchronisers.
-- The testbench drives it through bus procedures and checks timestamp deltas.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package reg_pkg is
  constant ADDR_ID      : natural := 16#00#;
  constant ADDR_CTRL    : natural := 16#04#;
  constant ADDR_STATUS  : natural := 16#08#;
  constant ADDR_SCRATCH : natural := 16#0C#;
  constant ADDR_TS_LO   : natural := 16#10#;
  constant ADDR_TS_HI   : natural := 16#14#;
  constant ADDR_CAP_LO  : natural := 16#18#;
  constant ADDR_CAP_HI  : natural := 16#1C#;
  constant ADDR_CMP_LO  : natural := 16#20#;
  constant ADDR_CMP_HI  : natural := 16#24#;
  constant ADDR_LD_LO   : natural := 16#28#;
  constant ADDR_LD_HI   : natural := 16#2C#;
  constant ADDR_RELOAD  : natural := 16#30#;
  constant ADDR_OVR     : natural := 16#34#;
  constant ADDR_BCNT    : natural := 16#38#;
  constant ADDR_PAR     : natural := 16#3C#;
  constant ADDR_MEM     : natural := 16#40#;

  type bus_req_t is record
    addr  : unsigned(7 downto 0);
    wdata : std_logic_vector(31 downto 0);
    wr    : std_logic;
    rd    : std_logic;
  end record;

  type bus_rsp_t is record
    ack   : std_logic;
    err   : std_logic;
    rdata : std_logic_vector(31 downto 0);
  end record;

  constant BUS_IDLE : bus_req_t :=
    (addr => (others => '0'), wdata => (others => '0'), wr => '0', rd => '0');

  type state_t is (S_IDLE, S_LATCHED, S_RAISE, S_WAIT_CLEAR);
  type word_array_t is array (natural range <>) of std_logic_vector(31 downto 0);

  function gray_encode(b : unsigned) return unsigned;
  function gray_decode(g : unsigned) return unsigned;
  function parity(v : std_logic_vector) return std_logic;
end package reg_pkg;

package body reg_pkg is
  function gray_encode(b : unsigned) return unsigned is
  begin
    return b xor shift_right(b, 1);
  end function;

  function gray_decode(g : unsigned) return unsigned is
    variable r : unsigned(g'range);
  begin
    r(g'left) := g(g'left);
    for i in g'left - 1 downto g'right loop
      r(i) := r(i + 1) xor g(i);
    end loop;
    return r;
  end function;

  function parity(v : std_logic_vector) return std_logic is
    variable p : std_logic := '0';
  begin
    for i in v'range loop
      p := p xor v(i);
    end loop;
    return p;
  end function;
end package body reg_pkg;

library ieee;
use ieee.std_logic_1164.all;

-- Flip-flop chain that moves one bit into the destination clock domain.
entity sync_bit is
  generic (
    stages : natural range 2 to 8 := 2;
    init   : std_logic := '0'
  );
  port (
    clk : in  std_logic;
    d   : in  std_logic;
    q   : out std_logic
  );
end entity sync_bit;

architecture rtl of sync_bit is
  signal chain : std_logic_vector(stages - 1 downto 0) := (others => init);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      chain <= chain(stages - 2 downto 0) & d;
    end if;
  end process;
  q <= chain(stages - 1);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.reg_pkg.all;

entity reg_block is
  port (
    clk_a  : in  std_logic;
    clk_b  : in  std_logic;
    rst_n  : in  std_logic;
    req    : in  bus_req_t;
    rsp    : out bus_rsp_t;
    ev_b   : in  std_logic;
    irq    : out std_logic;
    dbg_ts : out unsigned(63 downto 0)
  );
end entity reg_block;

architecture rtl of reg_block is
  component sync_bit is
    generic (stages : natural range 2 to 8 := 2; init : std_logic := '0');
    port (clk : in std_logic; d : in std_logic; q : out std_logic);
  end component;

  -- bus process state
  signal ctrl_en, ctrl_cmp_en, ctrl_cap_en, ctrl_irq_en : std_logic := '0';
  signal clear_p, load_p, update_p : std_logic := '0';
  signal w1c_p        : std_logic_vector(2 downto 0) := (others => '0');
  signal scratch      : std_logic_vector(31 downto 0) := (others => '0');
  signal ld_val       : unsigned(63 downto 0) := (others => '0');
  signal cmp_shadow   : unsigned(63 downto 0) := (others => '0');
  signal shadow_reload: unsigned(15 downto 0) := (others => '0');
  signal mem          : word_array_t(0 to 15) := (others => (others => '0'));
  signal ack, err     : std_logic := '0';
  signal rdata_r      : std_logic_vector(31 downto 0) := (others => '0');
  signal snap_hi      : std_logic_vector(31 downto 0) := (others => '0');
  signal status_bad   : std_logic := '0';

  -- timestamp
  signal ts            : unsigned(63 downto 0) := (others => '0');
  signal presc         : unsigned(15 downto 0) := (others => '0');
  signal active_reload : unsigned(15 downto 0) := (others => '0');

  -- event handling
  signal ev_sync, ev_prev : std_logic := '0';
  signal cap_val, cmp_val : unsigned(63 downto 0) := (others => '0');
  signal status_cap, status_cmp, status_ovr : std_logic := '0';
  signal ovr_cnt   : unsigned(7 downto 0) := (others => '0');
  signal state     : state_t := S_IDLE;
  signal latch_cap, latch_cmp : std_logic := '0';
  signal irq_r     : std_logic := '0';
  signal busy      : std_logic;

  -- second clock domain
  signal en_b      : std_logic;
  signal b_cnt     : unsigned(7 downto 0) := (others => '0');
  signal gray_b, gray_a : unsigned(7 downto 0) := (others => '0');
  signal gray_a_slv : std_logic_vector(7 downto 0);
  signal bcnt_a    : unsigned(7 downto 0);
begin
  -- Bus decode with asynchronous reset: one request per ack handshake.
  bus_p : process (clk_a, rst_n)
    variable a   : natural;
    variable idx : natural range 0 to 15;
    variable st  : std_logic_vector(31 downto 0);
  begin
    if rst_n = '0' then
      ack <= '0';
      err <= '0';
      rdata_r <= (others => '0');
      ctrl_en <= '0';
      ctrl_cmp_en <= '0';
      ctrl_cap_en <= '0';
      ctrl_irq_en <= '0';
      clear_p <= '0';
      load_p <= '0';
      update_p <= '0';
      w1c_p <= (others => '0');
      scratch <= (others => '0');
      status_bad <= '0';
      mem <= (others => (others => '0'));
    elsif rising_edge(clk_a) then
      clear_p <= '0';
      load_p <= '0';
      update_p <= '0';
      w1c_p <= (others => '0');
      ack <= '0';
      err <= '0';
      if ack = '0' and (req.wr = '1' or req.rd = '1') then
        ack <= '1';
        a := to_integer(req.addr);
        idx := to_integer(req.addr(5 downto 2));
        if req.wr = '1' then
          case a is
            when ADDR_CTRL =>
              ctrl_en     <= req.wdata(0);
              clear_p     <= req.wdata(1);
              load_p      <= req.wdata(2);
              ctrl_cmp_en <= req.wdata(3);
              ctrl_cap_en <= req.wdata(4);
              ctrl_irq_en <= req.wdata(5);
              update_p    <= req.wdata(6);
            when ADDR_STATUS =>
              w1c_p <= req.wdata(2 downto 0);
              if req.wdata(4) = '1' then
                status_bad <= '0';
              end if;
            when ADDR_SCRATCH =>
              scratch <= req.wdata;
            when ADDR_LD_LO =>
              ld_val(31 downto 0) <= unsigned(req.wdata);
            when ADDR_LD_HI =>
              ld_val(63 downto 32) <= unsigned(req.wdata);
            when ADDR_CMP_LO =>
              cmp_shadow(31 downto 0) <= unsigned(req.wdata);
            when ADDR_CMP_HI =>
              cmp_shadow(63 downto 32) <= unsigned(req.wdata);
            when ADDR_RELOAD =>
              shadow_reload <= unsigned(req.wdata(15 downto 0));
            when ADDR_MEM to ADDR_MEM + 60 =>
              mem(idx) <= req.wdata;
            when others =>
              err <= '1';
              status_bad <= '1';
          end case;
        else
          rdata_r <= (others => '0');
          st := (0 => status_cmp, 1 => status_cap, 2 => status_ovr,
                 3 => busy, 4 => status_bad, others => '0');
          case a is
            when ADDR_ID =>
              rdata_r <= x"5453_0102";
            when ADDR_CTRL =>
              rdata_r(6 downto 0) <= '0' & ctrl_irq_en & ctrl_cap_en & ctrl_cmp_en
                                     & '0' & '0' & ctrl_en;
            when ADDR_STATUS =>
              rdata_r <= st;
            when ADDR_SCRATCH =>
              rdata_r <= scratch;
            when ADDR_TS_LO =>
              rdata_r <= std_logic_vector(ts(31 downto 0));
              snap_hi <= std_logic_vector(ts(63 downto 32));
            when ADDR_TS_HI =>
              rdata_r <= snap_hi;
            when ADDR_CAP_LO =>
              rdata_r <= std_logic_vector(cap_val(31 downto 0));
            when ADDR_CAP_HI =>
              rdata_r <= std_logic_vector(cap_val(63 downto 32));
            when ADDR_CMP_LO | ADDR_CMP_HI =>
              rdata_r <= std_logic_vector(cmp_val(31 + 32 * ((a - ADDR_CMP_LO) / 4) downto 32 * ((a - ADDR_CMP_LO) / 4)));
            when ADDR_RELOAD =>
              rdata_r(15 downto 0) <= std_logic_vector(active_reload);
            when ADDR_OVR =>
              rdata_r(7 downto 0) <= std_logic_vector(ovr_cnt);
            when ADDR_BCNT =>
              rdata_r(7 downto 0) <= std_logic_vector(bcnt_a);
            when ADDR_PAR =>
              rdata_r(0) <= parity(scratch);
            when ADDR_MEM to ADDR_MEM + 60 =>
              rdata_r <= mem(idx);
            when others =>
              rdata_r <= x"DEAD_BEEF";
              err <= '1';
              status_bad <= '1';
          end case;
        end if;
      end if;
    end if;
  end process bus_p;

  rsp <= (ack => ack, err => err, rdata => rdata_r);

  -- Free-running timestamp with a prescaler and a shadowed reload value.
  ts_p : process (clk_a)
  begin
    if rising_edge(clk_a) then
      if rst_n = '0' then
        ts <= (others => '0');
        presc <= (others => '0');
        active_reload <= (others => '0');
      else
        if update_p = '1' then
          active_reload <= shadow_reload;
        end if;
        if clear_p = '1' then
          ts <= (others => '0');
          presc <= (others => '0');
        elsif load_p = '1' then
          ts <= ld_val;
        elsif ctrl_en = '1' then
          if presc = 0 then
            ts <= ts + 1;
            presc <= active_reload;
          else
            presc <= presc - 1;
          end if;
        end if;
      end if;
    end if;
  end process ts_p;

  dbg_ts <= ts;

  -- Capture, compare and the latched-event FSM.
  evt_p : process (clk_a)
    variable cap_hit, cmp_hit : boolean;
  begin
    if rising_edge(clk_a) then
      if rst_n = '0' then
        state <= S_IDLE;
        status_cap <= '0';
        status_cmp <= '0';
        status_ovr <= '0';
        ovr_cnt <= (others => '0');
        irq_r <= '0';
        latch_cap <= '0';
        latch_cmp <= '0';
      else
        ev_prev <= ev_sync;
        if update_p = '1' then
          cmp_val <= cmp_shadow;
        end if;
        cap_hit := (ev_sync xor ev_prev) = '1' and ctrl_cap_en = '1';
        cmp_hit := ctrl_cmp_en = '1' and ts = cmp_val;
        if cap_hit then
          cap_val <= ts;
        end if;
        if w1c_p(0) = '1' then status_cmp <= '0'; end if;
        if w1c_p(1) = '1' then status_cap <= '0'; end if;
        if w1c_p(2) = '1' then
          status_ovr <= '0';
          ovr_cnt <= (others => '0');
        end if;
        case state is
          when S_IDLE =>
            if cap_hit or cmp_hit then
              latch_cap <= '1' when cap_hit else '0';
              latch_cmp <= '1' when cmp_hit else '0';
              state <= S_LATCHED;
            end if;
          when S_LATCHED =>
            status_cap <= status_cap or latch_cap;
            status_cmp <= status_cmp or latch_cmp;
            state <= S_RAISE;
          when S_RAISE =>
            irq_r <= '1';
            state <= S_WAIT_CLEAR;
          when S_WAIT_CLEAR =>
            if (status_cap or status_cmp) = '0' then
              irq_r <= '0';
              state <= S_IDLE;
            elsif cap_hit or cmp_hit then
              status_ovr <= '1';
              if ovr_cnt /= 255 then
                ovr_cnt <= ovr_cnt + 1;
              end if;
            end if;
        end case;
      end if;
    end if;
  end process evt_p;

  irq <= irq_r when ctrl_irq_en = '1' else '0';

  with state select
    busy <= '0' when S_IDLE,
            '1' when others;

  -- Second clock domain: a gray-coded counter sent across bit by bit.
  u_ev : entity work.sync_bit
    generic map (stages => 2)
    port map (clk => clk_a, d => ev_b, q => ev_sync);

  u_en : sync_bit
    generic map (stages => 3)
    port map (clk => clk_b, d => ctrl_en, q => en_b);

  b_p : process (clk_b)
  begin
    if rising_edge(clk_b) then
      if en_b = '1' then
        b_cnt <= b_cnt + 1;
      end if;
      gray_b <= gray_encode(b_cnt);
    end if;
  end process b_p;

  g_gray : for i in gray_b'range generate
    u_bit : entity work.sync_bit(rtl)
      generic map (stages => 2)
      port map (clk => clk_a, d => gray_b(i), q => gray_a_slv(i));
  end generate g_gray;

  gray_a  <= unsigned(gray_a_slv);
  bcnt_a  <= gray_decode(gray_a);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.reg_pkg.all;

entity reg_timestamp_tb is
end entity reg_timestamp_tb;

architecture sim of reg_timestamp_tb is
  constant T_A : time := 10 ns;
  constant T_B : time := 7 ns;
  constant N_STRESS : natural := 8000;

  signal clk_a, clk_b : std_logic := '0';
  signal rst_n  : std_logic := '0';
  signal req    : bus_req_t := BUS_IDLE;
  signal rsp    : bus_rsp_t;
  signal ev_b   : std_logic := '0';
  signal irq    : std_logic;
  signal dbg_ts : unsigned(63 downto 0);
  signal done   : boolean := false;
  signal cyc    : natural := 0;
  signal mon_en : boolean := false;
  signal mon_cycles : natural := 0;

  procedure wait_cycles (signal clk : in std_logic; constant n : in natural) is
  begin
    for i in 1 to n loop
      wait until rising_edge(clk);
    end loop;
  end procedure;

  procedure bus_write (
    signal clk  : in  std_logic;
    signal req  : out bus_req_t;
    signal rsp  : in  bus_rsp_t;
    constant addr : in natural;
    constant data : in std_logic_vector(31 downto 0)) is
    variable tries : natural := 0;
  begin
    wait until rising_edge(clk);
    req <= (addr => to_unsigned(addr, 8), wdata => data, wr => '1', rd => '0');
    loop
      wait until rising_edge(clk);
      exit when rsp.ack = '1';
      tries := tries + 1;
      assert tries < 16 report "bus write timeout at address " & integer'image(addr)
        severity failure;
    end loop;
    req <= BUS_IDLE;
  end procedure;

  procedure bus_read (
    signal clk  : in  std_logic;
    signal req  : out bus_req_t;
    signal rsp  : in  bus_rsp_t;
    constant addr : in natural;
    variable data : out std_logic_vector(31 downto 0);
    variable err  : out std_logic) is
    variable tries : natural := 0;
  begin
    wait until rising_edge(clk);
    req <= (addr => to_unsigned(addr, 8), wdata => (others => '0'), wr => '0', rd => '1');
    loop
      wait until rising_edge(clk);
      exit when rsp.ack = '1';
      tries := tries + 1;
      assert tries < 16 report "bus read timeout at address " & integer'image(addr)
        severity failure;
    end loop;
    data := rsp.rdata;
    err := rsp.err;
    req <= BUS_IDLE;
  end procedure;

  procedure read64 (
    signal clk  : in  std_logic;
    signal req  : out bus_req_t;
    signal rsp  : in  bus_rsp_t;
    constant lo_addr : in natural;
    variable v : out unsigned(63 downto 0)) is
    variable lo, hi : std_logic_vector(31 downto 0);
    variable e : std_logic;
  begin
    bus_read(clk, req, rsp, lo_addr, lo, e);
    bus_read(clk, req, rsp, lo_addr + 4, hi, e);
    v := unsigned(hi) & unsigned(lo);
  end procedure;

  procedure write64 (
    signal clk  : in  std_logic;
    signal req  : out bus_req_t;
    signal rsp  : in  bus_rsp_t;
    constant lo_addr : in natural;
    constant v : in unsigned(63 downto 0)) is
  begin
    bus_write(clk, req, rsp, lo_addr, std_logic_vector(v(31 downto 0)));
    bus_write(clk, req, rsp, lo_addr + 4, std_logic_vector(v(63 downto 32)));
  end procedure;

  function next_rand (s : unsigned(31 downto 0)) return unsigned is
    variable x : unsigned(31 downto 0) := s;
  begin
    x := x xor shift_left(x, 13);
    x := x xor shift_right(x, 17);
    x := x xor shift_left(x, 5);
    return x;
  end function;

  function ones (v : std_logic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;
begin
  clk_a_p : process
  begin
    while not done loop
      clk_a <= '0';
      wait for T_A / 2;
      clk_a <= '1';
      wait for T_A / 2;
    end loop;
    wait;
  end process;

  clk_b <= not clk_b after T_B / 2 when not done else clk_b;

  dut : entity work.reg_block
    port map (clk_a => clk_a, clk_b => clk_b, rst_n => rst_n, req => req,
              rsp => rsp, ev_b => ev_b, irq => irq, dbg_ts => dbg_ts);

  -- Event source in the second clock domain: a toggle every 64 cycles.
  ev_gen : process (clk_b)
    variable n : natural := 0;
  begin
    if rising_edge(clk_b) then
      n := n + 1;
      if n = 64 then
        n := 0;
        ev_b <= not ev_b;
      end if;
    end if;
  end process;

  cyc_p : process (clk_a)
  begin
    if rising_edge(clk_a) then
      cyc <= cyc + 1;
    end if;
  end process;

  -- While enabled, the timestamp must step by exactly one every cycle.
  mon_p : process (clk_a)
    variable prev, d : unsigned(63 downto 0) := (others => '0');
  begin
    if rising_edge(clk_a) then
      d := dbg_ts - prev;
      prev := dbg_ts;
      if mon_en then
        assert d = 1
          report "timestamp step " & integer'image(to_integer(d(15 downto 0)))
          severity failure;
        mon_cycles <= mon_cycles + 1;
      end if;
    end if;
  end process;

  main : process
    variable rd, w : std_logic_vector(31 downto 0);
    variable e : std_logic;
    variable t0, t1, tgt, cap_prev, cap : unsigned(63 downto 0);
    variable c0, c1, n, expect : natural;
    variable b0, b1 : unsigned(7 downto 0);
    variable r : unsigned(31 downto 0) := x"1234_5678";
    variable mem_model : word_array_t(0 to 15) := (others => (others => '0'));
    variable cmp_count, ts_count, mem_count, scr_count : natural := 0;
  begin
    rst_n <= '0';
    wait_cycles(clk_a, 5);
    rst_n <= '1';
    wait_cycles(clk_a, 3);

    -- identity and scratch register
    bus_read(clk_a, req, rsp, ADDR_ID, rd, e);
    assert rd = x"54530102" and e = '0'
      report "bad id " & to_hstring(rd) severity failure;
    for i in 0 to 31 loop
      w := std_logic_vector(shift_left(to_unsigned(1, 32), i));
      bus_write(clk_a, req, rsp, ADDR_SCRATCH, w);
      bus_read(clk_a, req, rsp, ADDR_SCRATCH, rd, e);
      assert rd = w report "scratch walk " & integer'image(i) severity failure;
      bus_read(clk_a, req, rsp, ADDR_PAR, rd, e);
      assert rd = x"00000001" report "parity of one bit" severity failure;
    end loop;
    for i in 1 to 16 loop
      r := next_rand(r);
      bus_write(clk_a, req, rsp, ADDR_SCRATCH, std_logic_vector(r));
      bus_read(clk_a, req, rsp, ADDR_PAR, rd, e);
      assert (rd(0) = '1') = (ones(std_logic_vector(r)) mod 2 = 1)
        report "parity mismatch" severity failure;
    end loop;

    -- memory window and unmapped address
    for i in 0 to 15 loop
      r := next_rand(r);
      mem_model(i) := std_logic_vector(r);
      bus_write(clk_a, req, rsp, ADDR_MEM + 4 * i, mem_model(i));
    end loop;
    for i in 15 downto 0 loop
      bus_read(clk_a, req, rsp, ADDR_MEM + 4 * i, rd, e);
      assert rd = mem_model(i) report "mem word " & integer'image(i) severity failure;
    end loop;
    bus_read(clk_a, req, rsp, 16#80#, rd, e);
    assert e = '1' and rd = x"DEADBEEF" report "unmapped read" severity failure;
    bus_read(clk_a, req, rsp, ADDR_STATUS, rd, e);
    assert rd(4) = '1' report "bad address flag" severity failure;
    bus_write(clk_a, req, rsp, ADDR_STATUS, x"00000010");
    bus_read(clk_a, req, rsp, ADDR_STATUS, rd, e);
    assert rd = x"00000000" report "status after clear " & to_hstring(rd) severity failure;

    -- free running timestamp: exact delta against the cycle counter
    bus_write(clk_a, req, rsp, ADDR_RELOAD, x"00000000");
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000043");
    wait_cycles(clk_a, 4);
    mon_en <= true;
    for k in 1 to 12 loop
      c0 := cyc;
      read64(clk_a, req, rsp, ADDR_TS_LO, t0);
      wait_cycles(clk_a, 7 * k);
      c1 := cyc;
      read64(clk_a, req, rsp, ADDR_TS_LO, t1);
      assert to_integer(t1 - t0) = c1 - c0
        report "delta " & integer'image(to_integer(t1 - t0)) & " vs cycles "
               & integer'image(c1 - c0) severity failure;
    end loop;
    mon_en <= false;
    wait_cycles(clk_a, 2);

    -- prescaler: one tick every four cycles
    bus_write(clk_a, req, rsp, ADDR_RELOAD, x"00000003");
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000043");
    wait_cycles(clk_a, 10);
    c0 := cyc;
    read64(clk_a, req, rsp, ADDR_TS_LO, t0);
    wait_cycles(clk_a, 400);
    c1 := cyc;
    read64(clk_a, req, rsp, ADDR_TS_LO, t1);
    expect := (c1 - c0) / 4;
    assert abs(to_integer(t1 - t0) - expect) <= 1
      report "prescaled delta " & integer'image(to_integer(t1 - t0)) severity failure;

    -- stopped counter does not move
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000000");
    wait_cycles(clk_a, 6);
    read64(clk_a, req, rsp, ADDR_TS_LO, t0);
    wait_cycles(clk_a, 50);
    read64(clk_a, req, rsp, ADDR_TS_LO, t1);
    assert t0 = t1 report "stopped counter moved" severity failure;

    -- preload near the 32-bit and the 64-bit wrap points
    bus_write(clk_a, req, rsp, ADDR_RELOAD, x"00000000");
    write64(clk_a, req, rsp, ADDR_LD_LO, x"0000_0000_FFFF_FF80");
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000045");
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000041");
    wait_cycles(clk_a, 300);
    read64(clk_a, req, rsp, ADDR_TS_LO, t0);
    assert t0(63 downto 32) = x"00000001" and t0(31 downto 0) < 300
      report "32-bit carry" severity failure;
    write64(clk_a, req, rsp, ADDR_LD_LO, x"FFFF_FFFF_FFFF_FF80");
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000045");
    wait_cycles(clk_a, 2);
    mon_en <= true;
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000041");
    wait_cycles(clk_a, 300);
    read64(clk_a, req, rsp, ADDR_TS_LO, t0);
    assert t0 < 400 and t0 > 100 report "64-bit wrap" severity failure;
    mon_en <= false;

    -- capture of events from the other clock domain
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000071");
    cap_prev := (others => '0');
    n := 0;
    for k in 1 to 40 loop
      wait until irq = '1' for 3 us;
      assert irq = '1' report "no capture irq" severity failure;
      bus_read(clk_a, req, rsp, ADDR_STATUS, rd, e);
      assert rd(1) = '1' and rd(3) = '1' report "capture status" severity failure;
      read64(clk_a, req, rsp, ADDR_CAP_LO, cap);
      if k > 1 then
        assert to_integer(cap - cap_prev) >= 43 and to_integer(cap - cap_prev) <= 47
          report "capture spacing " & integer'image(to_integer(cap - cap_prev))
          severity failure;
        n := n + 1;
      end if;
      cap_prev := cap;
      bus_write(clk_a, req, rsp, ADDR_STATUS, x"00000002");
      wait_cycles(clk_a, 4);
      assert irq = '0' report "irq stuck" severity failure;
    end loop;
    -- leave events unserviced so that overruns are counted
    wait until irq = '1' for 3 us;
    wait_cycles(clk_a, 400);
    bus_read(clk_a, req, rsp, ADDR_OVR, rd, e);
    assert unsigned(rd) >= 5 report "overrun count " & to_hstring(rd) severity failure;
    bus_read(clk_a, req, rsp, ADDR_STATUS, rd, e);
    assert rd(2) = '1' report "overrun flag" severity failure;
    bus_write(clk_a, req, rsp, ADDR_STATUS, x"00000007");
    bus_read(clk_a, req, rsp, ADDR_OVR, rd, e);
    assert rd = x"00000000" report "overrun clear" severity failure;
    bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000069");

    -- counter crossing into the other clock domain
    b0 := (others => '0');
    for k in 1 to 20 loop
      c0 := cyc;
      bus_read(clk_a, req, rsp, ADDR_BCNT, rd, e);
      b1 := unsigned(rd(7 downto 0));
      wait_cycles(clk_a, 25);
      if k > 1 then
        c1 := cyc;
        expect := ((c0 - n) * 10) / 7;
        assert (b1 - b0) /= 0 report "b counter stuck" severity failure;
      end if;
      b0 := b1;
      n := c0;
    end loop;

    -- mixed traffic with random choice of operation
    mon_en <= true;
    for k in 1 to N_STRESS loop
      r := next_rand(r);
      case to_integer(r(1 downto 0)) is
        when 0 =>
          bus_write(clk_a, req, rsp, ADDR_SCRATCH, std_logic_vector(r));
          bus_read(clk_a, req, rsp, ADDR_SCRATCH, rd, e);
          assert rd = std_logic_vector(r) report "stress scratch" severity failure;
          scr_count := scr_count + 1;
        when 1 =>
          n := to_integer(r(7 downto 4));
          mem_model(n) := std_logic_vector(r(15 downto 0)) & std_logic_vector(r(31 downto 16));
          bus_write(clk_a, req, rsp, ADDR_MEM + 4 * n, mem_model(n));
          bus_read(clk_a, req, rsp, ADDR_MEM + 4 * n, rd, e);
          assert rd = mem_model(n) report "stress mem" severity failure;
          mem_count := mem_count + 1;
        when 2 =>
          c0 := cyc;
          read64(clk_a, req, rsp, ADDR_TS_LO, t0);
          wait_cycles(clk_a, to_integer(r(11 downto 8)));
          c1 := cyc;
          read64(clk_a, req, rsp, ADDR_TS_LO, t1);
          assert to_integer(t1 - t0) = c1 - c0 report "stress delta" severity failure;
          ts_count := ts_count + 1;
        when others =>
          mon_en <= false;
          read64(clk_a, req, rsp, ADDR_TS_LO, t0);
          tgt := t0 + 80 + resize(r(13 downto 8), 64);
          write64(clk_a, req, rsp, ADDR_CMP_LO, tgt);
          bus_write(clk_a, req, rsp, ADDR_CTRL, x"00000069");
          mon_en <= true;
          wait until irq = '1' for 5 us;
          assert irq = '1' report "no compare irq" severity failure;
          bus_read(clk_a, req, rsp, ADDR_STATUS, rd, e);
          assert rd(0) = '1' and rd(3) = '1' report "compare status" severity failure;
          read64(clk_a, req, rsp, ADDR_TS_LO, t1);
          assert t1 >= tgt and t1 - tgt < 30 report "compare timing" severity failure;
          bus_write(clk_a, req, rsp, ADDR_STATUS, x"00000001");
          wait_cycles(clk_a, 5);
          assert irq = '0' report "compare irq stuck" severity failure;
          cmp_count := cmp_count + 1;
      end case;
    end loop;
    mon_en <= false;

    assert mon_cycles > 1000 report "monitor ran too little" severity failure;
    report "ops: scratch=" & integer'image(scr_count) & " mem=" & integer'image(mem_count)
           & " ts=" & integer'image(ts_count) & " cmp=" & integer'image(cmp_count);
    report "PASS" severity note;
    done <= true;
    wait;
  end process main;
end architecture sim;
