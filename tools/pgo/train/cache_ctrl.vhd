-- pgo: top=cache_ctrl_tb std=2008 stop=1700 us
-- Reproduces a 4 way, 64 set, 8 word line write-back cache controller with record
-- tag RAM, data RAM, tree replacement bits, a latency memory model, a random
-- requester and a shadow memory scoreboard, as several small clocked processes.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package cc_pkg is
  constant WAYS    : natural := 4;
  constant SETS    : natural := 64;
  constant LINE    : natural := 8;
  constant ADDR_W  : natural := 14;
  constant OFF_W   : natural := 3;
  constant IDX_W   : natural := 6;
  constant TAG_W   : natural := ADDR_W - IDX_W - OFF_W;
  constant MEM_WORDS : natural := 2 ** ADDR_W;

  subtype word_t is std_logic_vector(31 downto 0);
  subtype addr_t is unsigned(ADDR_W - 1 downto 0);
  subtype tag_t is unsigned(TAG_W - 1 downto 0);
  subtype way_t is natural range 0 to WAYS - 1;
  subtype set_t is natural range 0 to SETS - 1;
  subtype data_idx_t is natural range 0 to SETS * WAYS * LINE - 1;
  subtype be_t is std_logic_vector(3 downto 0);

  type tag_entry_t is record
    valid : std_ulogic;
    dirty : std_ulogic;
    tag   : tag_t;
  end record;
  constant TAG_INVALID : tag_entry_t := (valid => '0', dirty => '0', tag => (others => '0'));

  type tag_set_arr_t is array (0 to WAYS - 1) of tag_entry_t;
  type tag_ram_t is array (0 to SETS - 1) of tag_set_arr_t;
  type data_ram_t is array (0 to SETS * WAYS * LINE - 1) of word_t;
  type plru_ram_t is array (0 to SETS - 1) of std_logic_vector(2 downto 0);
  type mem_t is array (0 to MEM_WORDS - 1) of word_t;

  type req_t is record
    valid : std_ulogic;
    we    : std_ulogic;
    addr  : addr_t;
    wdata : word_t;
    be    : be_t;
  end record;
  constant REQ_IDLE : req_t := ('0', '0', (others => '0'), (others => '0'), (others => '0'));

  type resp_t is record
    valid : std_ulogic;
    rdata : word_t;
  end record;
  constant RESP_IDLE : resp_t := ('0', (others => '0'));

  type mem_req_t is record
    valid : std_ulogic;
    we    : std_ulogic;
    addr  : addr_t;
    wdata : word_t;
  end record;
  constant MEM_REQ_IDLE : mem_req_t := ('0', '0', (others => '0'), (others => '0'));

  type mem_rsp_t is record
    ack   : std_ulogic;
    rdata : word_t;
  end record;

  type data_wr_t is record
    we   : std_ulogic;
    idx  : data_idx_t;
    data : word_t;
    be   : be_t;
  end record;
  constant DW_IDLE : data_wr_t := ('0', 0, (others => '0'), (others => '0'));

  type tag_wr_t is record
    we    : std_ulogic;
    set   : set_t;
    way   : way_t;
    entry : tag_entry_t;
  end record;
  constant TW_IDLE : tag_wr_t := ('0', 0, 0, TAG_INVALID);

  type plru_upd_t is record
    en  : std_ulogic;
    set : set_t;
    way : way_t;
  end record;
  constant PU_IDLE : plru_upd_t := ('0', 0, 0);

  type stats_t is record
    hits, misses, wbs, refills : unsigned(31 downto 0);
  end record;

  function set_of(a : addr_t) return set_t;
  function off_of(a : addr_t) return natural;
  function tag_of(a : addr_t) return tag_t;
  function line_index(s : set_t; w : way_t; o : natural) return data_idx_t;
  function plru_victim(b : std_logic_vector(2 downto 0)) return way_t;
  function plru_touch(b : std_logic_vector(2 downto 0); w : way_t) return std_logic_vector;
  function xorshift(x : unsigned(31 downto 0)) return unsigned;
  function init_mem return mem_t;
  procedure merge(w : inout word_t; wdata : in word_t; be : in be_t);
end package;

package body cc_pkg is
  function set_of(a : addr_t) return set_t is
  begin
    return to_integer(a(OFF_W + IDX_W - 1 downto OFF_W));
  end function;

  function off_of(a : addr_t) return natural is
  begin
    return to_integer(a(OFF_W - 1 downto 0));
  end function;

  function tag_of(a : addr_t) return tag_t is
  begin
    return a(ADDR_W - 1 downto ADDR_W - TAG_W);
  end function;

  function line_index(s : set_t; w : way_t; o : natural) return data_idx_t is
  begin
    return (s * WAYS + w) * LINE + o;
  end function;

  -- Tree replacement: bit 0 picks the pair, bits 1 and 2 pick the way inside it.
  function plru_victim(b : std_logic_vector(2 downto 0)) return way_t is
  begin
    if b(0) = '0' then
      if b(1) = '0' then
        return 0;
      end if;
      return 1;
    end if;
    if b(2) = '0' then
      return 2;
    end if;
    return 3;
  end function;

  function plru_touch(b : std_logic_vector(2 downto 0); w : way_t) return std_logic_vector is
    variable n : std_logic_vector(2 downto 0) := b;
  begin
    case w is
      when 0 to 1 =>
        n(0) := '1';
        if w = 0 then
          n(1) := '1';
        else
          n(1) := '0';
        end if;
      when 2 to 3 =>
        n(0) := '0';
        if w = 2 then
          n(2) := '1';
        else
          n(2) := '0';
        end if;
    end case;
    return n;
  end function;

  function xorshift(x : unsigned(31 downto 0)) return unsigned is
    variable y : unsigned(31 downto 0) := x;
  begin
    y := y xor shift_left(y, 13);
    y := y xor shift_right(y, 17);
    y := y xor shift_left(y, 5);
    return y;
  end function;

  function init_mem return mem_t is
    variable m : mem_t;
    constant K : unsigned(31 downto 0) := x"9E3779B1";
  begin
    for i in m'range loop
      m(i) := std_logic_vector(resize(to_unsigned(i, 32) * K, 32) xor to_unsigned(i, 32));
    end loop;
    return m;
  end function;

  procedure merge(w : inout word_t; wdata : in word_t; be : in be_t) is
  begin
    for b in be'reverse_range loop
      if be(b) = '1' then
        w(8 * b + 7 downto 8 * b) := wdata(8 * b + 7 downto 8 * b);
      end if;
    end loop;
  end procedure;
end package body;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cc_pkg.all;

entity cc_mem is
  generic (g_latency : natural range 0 to 15 := 3);
  port (
    clk, rst : in  std_ulogic;
    req      : in  mem_req_t;
    rsp      : out mem_rsp_t
  );
end entity;

architecture sim of cc_mem is
  signal ack_q : std_ulogic := '0';
  signal rd_q  : word_t := (others => '0');
begin
  rsp <= (ack => ack_q, rdata => rd_q);

  -- A request is served after g_latency wait cycles; ack lasts one cycle.
  serve : process (clk)
    variable mem : mem_t := init_mem;
    variable cnt : natural range 0 to 15 := 0;
    variable a   : natural;
  begin
    if rising_edge(clk) then
      ack_q <= '0';
      if rst = '1' then
        cnt := 0;
      elsif req.valid = '1' and ack_q = '0' then
        if cnt < g_latency then
          cnt := cnt + 1;
        else
          a := to_integer(req.addr);
          if req.we = '1' then
            mem(a) := req.wdata;
          else
            rd_q <= mem(a);
          end if;
          ack_q <= '1';
          cnt := 0;
        end if;
      end if;
    end if;
  end process;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cc_pkg.all;

entity cc_ctrl is
  port (
    clk, rst : in  std_ulogic;
    req      : in  req_t;
    ready    : out std_ulogic;
    resp     : out resp_t;
    mreq     : out mem_req_t;
    mrsp     : in  mem_rsp_t;
    stats    : out stats_t
  );
end entity;

architecture rtl of cc_ctrl is
  type state_t is (S_IDLE, S_LOOKUP, S_RESP, S_WB_RD, S_WB_REQ, S_RF_REQ);
  type reg_t is record
    st      : state_t;
    req     : req_t;
    way     : way_t;
    cnt     : unsigned(3 downto 0);
    retry   : std_ulogic;
    vic_tag : tag_t;
  end record;
  constant R_INIT : reg_t := (S_IDLE, REQ_IDLE, 0, (others => '0'), '0', (others => '0'));
  signal r, rin : reg_t := R_INIT;

  signal tags   : tag_ram_t := (others => (others => TAG_INVALID));
  signal data   : data_ram_t := (others => (others => '0'));
  signal plru   : plru_ram_t := (others => (others => '0'));
  signal dq     : word_t := (others => '0');
  signal dr_idx : data_idx_t := 0;
  signal dw     : data_wr_t := DW_IDLE;
  signal tw     : tag_wr_t := TW_IDLE;
  signal pu     : plru_upd_t := PU_IDLE;
  signal hit    : boolean := false;
  signal hit_way, victim : way_t := 0;
  signal ev_hit, ev_miss, ev_wb, ev_rf : std_ulogic := '0';
  signal st_q   : stats_t := ((others => '0'), (others => '0'), (others => '0'), (others => '0'));
begin
  -- Tag compare across the ways and victim choice (an invalid way wins).
  lookup : process (all)
    variable s   : set_t;
    variable h   : boolean;
    variable hw  : way_t;
    variable vic : way_t;
  begin
    s := set_of(r.req.addr);
    h := false;
    hw := 0;
    for w in 0 to WAYS - 1 loop
      if tags(s)(w).valid = '1' and tags(s)(w).tag = tag_of(r.req.addr) then
        h := true;
        hw := w;
      end if;
    end loop;
    vic := plru_victim(plru(s));
    for w in WAYS - 1 downto 0 loop
      if tags(s)(w).valid = '0' then
        vic := w;
      end if;
    end loop;
    hit <= h;
    hit_way <= hw;
    victim <= vic;
  end process;

  fsm : process (all)
    variable v     : reg_t;
    variable s     : set_t;
    variable o     : natural range 0 to LINE - 1;
    variable rdy   : std_ulogic;
    variable rs    : resp_t;
    variable m     : mem_req_t;
    variable dw_v  : data_wr_t;
    variable tw_v  : tag_wr_t;
    variable pu_v  : plru_upd_t;
    variable dri   : data_idx_t;
    variable ent   : tag_entry_t;
    variable e_h, e_m, e_w, e_r : std_ulogic;
  begin
    v := r;
    s := set_of(r.req.addr);
    o := off_of(r.req.addr);
    rdy := '0';
    rs := RESP_IDLE;
    m := MEM_REQ_IDLE;
    dw_v := DW_IDLE;
    tw_v := TW_IDLE;
    pu_v := PU_IDLE;
    dri := line_index(s, r.way, o);
    e_h := '0';
    e_m := '0';
    e_w := '0';
    e_r := '0';
    case r.st is
      when S_IDLE =>
        rdy := '1';
        if req.valid = '1' then
          v.req := req;
          v.retry := '0';
          v.st := S_LOOKUP;
        end if;

      when S_LOOKUP =>
        if hit then
          v.way := hit_way;
          dri := line_index(s, hit_way, o);
          if r.req.we = '1' then
            dw_v := ('1', dri, r.req.wdata, r.req.be);
            ent := tags(s)(hit_way);
            ent.dirty := '1';
            tw_v := ('1', s, hit_way, ent);
          end if;
          pu_v := ('1', s, hit_way);
          e_h := not r.retry;
          v.st := S_RESP;
        else
          v.way := victim;
          v.vic_tag := tags(s)(victim).tag;
          v.cnt := (others => '0');
          e_m := '1';
          if tags(s)(victim).valid = '1' and tags(s)(victim).dirty = '1' then
            v.st := S_WB_RD;
          else
            v.st := S_RF_REQ;
          end if;
        end if;

      when S_RESP =>
        rs := ('1', dq);
        v.st := S_IDLE;

      when S_WB_RD =>
        dri := line_index(s, r.way, to_integer(r.cnt(OFF_W - 1 downto 0)));
        v.st := S_WB_REQ;

      when S_WB_REQ =>
        dri := line_index(s, r.way, to_integer(r.cnt(OFF_W - 1 downto 0)));
        m := ('1', '1', r.vic_tag & to_unsigned(s, IDX_W) & r.cnt(OFF_W - 1 downto 0), dq);
        if mrsp.ack = '1' then
          if r.cnt = LINE - 1 then
            v.cnt := (others => '0');
            e_w := '1';
            v.st := S_RF_REQ;
          else
            v.cnt := r.cnt + 1;
            v.st := S_WB_RD;
          end if;
        end if;

      when S_RF_REQ =>
        m := ('1', '0', tag_of(r.req.addr) & to_unsigned(s, IDX_W) & r.cnt(OFF_W - 1 downto 0),
              (others => '0'));
        if mrsp.ack = '1' then
          dw_v := ('1', line_index(s, r.way, to_integer(r.cnt(OFF_W - 1 downto 0))),
                   mrsp.rdata, "1111");
          if r.cnt = LINE - 1 then
            tw_v := ('1', s, r.way, ('1', '0', tag_of(r.req.addr)));
            v.retry := '1';
            e_r := '1';
            v.st := S_LOOKUP;
          else
            v.cnt := r.cnt + 1;
          end if;
        end if;
    end case;
    if rst = '1' then
      v := R_INIT;
    end if;
    rin <= v;
    ready <= rdy;
    resp <= rs;
    mreq <= m;
    dw <= dw_v;
    tw <= tw_v;
    pu <= pu_v;
    dr_idx <= dri;
    ev_hit <= e_h;
    ev_miss <= e_m;
    ev_wb <= e_w;
    ev_rf <= e_r;
  end process;

  fsm_seq : process (clk)
  begin
    if rising_edge(clk) then
      assert not (r.st = S_LOOKUP and r.retry = '1' and not hit)
        report "refill did not produce a hit" severity failure;
      r <= rin;
    end if;
  end process;

  tag_ram : process (clk)
  begin
    if rising_edge(clk) then
      if tw.we = '1' then
        tags(tw.set)(tw.way) <= tw.entry;
      end if;
    end if;
  end process;

  data_ram : process (clk)
  begin
    if rising_edge(clk) then
      if dw.we = '1' then
        for b in 0 to 3 loop
          if dw.be(b) = '1' then
            data(dw.idx)(8 * b + 7 downto 8 * b) <= dw.data(8 * b + 7 downto 8 * b);
          end if;
        end loop;
      end if;
      dq <= data(dr_idx);
    end if;
  end process;

  plru_ram : process (clk)
  begin
    if rising_edge(clk) then
      if pu.en = '1' then
        plru(pu.set) <= plru_touch(plru(pu.set), pu.way);
      end if;
    end if;
  end process;

  counters : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        st_q <= ((others => '0'), (others => '0'), (others => '0'), (others => '0'));
      else
        if ev_hit = '1' then
          st_q.hits <= st_q.hits + 1;
        end if;
        if ev_miss = '1' then
          st_q.misses <= st_q.misses + 1;
        end if;
        if ev_wb = '1' then
          st_q.wbs <= st_q.wbs + 1;
        end if;
        if ev_rf = '1' then
          st_q.refills <= st_q.refills + 1;
        end if;
      end if;
    end if;
  end process;

  stats <= st_q;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cc_pkg.all;

entity cc_requester is
  generic (
    g_seed : std_logic_vector(31 downto 0);
    g_ops  : natural
  );
  port (
    clk, rst : in  std_ulogic;
    req      : out req_t;
    ready    : in  std_ulogic;
    resp     : in  resp_t;
    done     : out std_ulogic;
    checksum : out word_t
  );
end entity;

architecture rtl of cc_requester is
  type rstate_t is (R_GAP, R_REQ, R_WAIT, R_DONE);
  signal st : rstate_t := R_GAP;
begin
  gen : process (clk)
    variable x       : unsigned(31 downto 0) := unsigned(g_seed);
    variable last    : addr_t := (others => '0');
    variable a       : addr_t;
    variable gap     : natural range 0 to 7 := 0;
    variable cnt     : natural := 0;
    variable sum     : unsigned(31 downto 0) := (others => '0');
    variable pend_we : std_ulogic := '0';
    variable q       : req_t := REQ_IDLE;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        st <= R_GAP;
        q := REQ_IDLE;
        cnt := 0;
        sum := (others => '0');
        x := unsigned(g_seed);
      else
        case st is
          when R_GAP =>
            if gap = 0 then
              x := xorshift(x);
              case to_integer(x(2 downto 0)) is
                when 0 to 3 => a := last + resize(x(5 downto 4), ADDR_W);
                when 4 | 5  => a := last(ADDR_W - 1 downto OFF_W) & x(10 downto 8);
                when 6      => a := x(25 downto 12);
                when others => a := x(24 downto 20) & last(ADDR_W - TAG_W - 1 downto 0);
              end case;
              last := a;
              x := xorshift(x);
              q.valid := '1';
              q.addr := a;
              if x(15 downto 14) = "00" then
                q.we := '1';
              else
                q.we := '0';
              end if;
              q.wdata := std_logic_vector(x);
              q.be := std_logic_vector(x(19 downto 16));
              if q.be = "0000" then
                q.be := "1111";
              end if;
              pend_we := q.we;
              st <= R_REQ;
            else
              gap := gap - 1;
            end if;
          when R_REQ =>
            if ready = '1' then
              q.valid := '0';
              st <= R_WAIT;
            end if;
          when R_WAIT =>
            if resp.valid = '1' then
              if pend_we = '0' then
                sum := rotate_left(sum, 1) xor unsigned(resp.rdata);
              end if;
              cnt := cnt + 1;
              gap := to_integer(x(22 downto 20));
              if cnt = g_ops then
                st <= R_DONE;
              else
                st <= R_GAP;
              end if;
            end if;
          when R_DONE =>
            null;
        end case;
      end if;
      req <= q;
      checksum <= std_logic_vector(sum);
    end if;
  end process;

  done <= '1' when st = R_DONE else '0';
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cc_pkg.all;

entity cc_scoreboard is
  port (
    clk, rst : in  std_ulogic;
    req      : in  req_t;
    ready    : in  std_ulogic;
    resp     : in  resp_t;
    errors   : out natural;
    reads    : out natural;
    checksum : out word_t
  );
end entity;

architecture sim of cc_scoreboard is
begin
  -- Shadow memory: every accepted write updates it, every read response is compared.
  watch : process (clk)
    variable shadow  : mem_t := init_mem;
    variable pend    : boolean := false;
    variable pend_we : std_ulogic := '0';
    variable expv    : word_t := (others => '0');
    variable nerr    : natural := 0;
    variable nrd     : natural := 0;
    variable sum     : unsigned(31 downto 0) := (others => '0');
    variable a       : natural;
  begin
    if rising_edge(clk) then
      if resp.valid = '1' and pend then
        if pend_we = '0' then
          if resp.rdata /= expv then
            nerr := nerr + 1;
            report "read mismatch: got " & to_hstring(resp.rdata) & " want " & to_hstring(expv)
              severity error;
          end if;
          nrd := nrd + 1;
          sum := rotate_left(sum, 1) xor unsigned(expv);
        end if;
        pend := false;
      end if;
      if req.valid = '1' and ready = '1' then
        a := to_integer(req.addr);
        pend := true;
        pend_we := req.we;
        if req.we = '1' then
          merge(shadow(a), req.wdata, req.be);
        else
          expv := shadow(a);
        end if;
      end if;
      errors <= nerr;
      reads <= nrd;
      checksum <= std_logic_vector(sum);
    end if;
  end process;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cc_pkg.all;

entity cc_sys is
  generic (
    g_seed    : std_logic_vector(31 downto 0);
    g_ops     : natural;
    g_latency : natural range 0 to 15
  );
  port (
    clk, rst : in  std_ulogic;
    done     : out std_ulogic;
    errors   : out natural;
    reads    : out natural;
    sum_req  : out word_t;
    sum_sb   : out word_t;
    stats    : out stats_t
  );
end entity;

architecture rtl of cc_sys is
  signal req   : req_t;
  signal ready : std_ulogic;
  signal resp  : resp_t;
  signal mreq  : mem_req_t;
  signal mrsp  : mem_rsp_t;
  signal done_i : std_ulogic;
begin
  done <= done_i;

  u_req : entity work.cc_requester
    generic map (g_seed => g_seed, g_ops => g_ops)
    port map (clk => clk, rst => rst, req => req, ready => ready, resp => resp,
              done => done_i, checksum => sum_req);

  u_ctrl : entity work.cc_ctrl
    port map (clk => clk, rst => rst, req => req, ready => ready, resp => resp,
              mreq => mreq, mrsp => mrsp, stats => stats);

  u_mem : entity work.cc_mem
    generic map (g_latency => g_latency)
    port map (clk => clk, rst => rst, req => mreq, rsp => mrsp);

  u_sb : entity work.cc_scoreboard
    port map (clk => clk, rst => rst, req => req, ready => ready, resp => resp,
              errors => errors, reads => reads, checksum => sum_sb);

  -- No transaction may take more than a few hundred cycles.
  dog : process (clk)
    variable idle : natural := 0;
  begin
    if rising_edge(clk) then
      if resp.valid = '1' or rst = '1' or done_i = '1' then
        idle := 0;
      else
        idle := idle + 1;
      end if;
      assert idle < 600 report "controller hung" severity failure;
    end if;
  end process;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cc_pkg.all;

entity cache_ctrl_tb is
end entity;

architecture sim of cache_ctrl_tb is
  constant NSYS : natural := 2;
  type words_t is array (0 to NSYS - 1) of word_t;
  type nats_t is array (0 to NSYS - 1) of natural;
  type stats_arr_t is array (0 to NSYS - 1) of stats_t;
  type seeds_t is array (0 to NSYS - 1) of word_t;
  constant SEEDS : seeds_t := (x"1badf00d", x"c0ffee11");
  constant OPS   : nats_t := (6600, 4400);

  signal clk    : std_ulogic := '0';
  signal rst    : std_ulogic := '1';
  signal fin    : boolean := false;
  signal dones  : std_ulogic_vector(0 to NSYS - 1);
  signal errs   : nats_t;
  signal reads  : nats_t;
  signal sum_rq : words_t;
  signal sum_sb : words_t;
  signal stats  : stats_arr_t;
begin
  clk <= not clk after 5 ns when not fin else '0';
  rst <= '0' after 33 ns;

  systems : for i in 0 to NSYS - 1 generate
    u_sys : entity work.cc_sys
      generic map (g_seed => SEEDS(i), g_ops => OPS(i), g_latency => 2 + 3 * i)
      port map (clk => clk, rst => rst, done => dones(i), errors => errs(i), reads => reads(i),
                sum_req => sum_rq(i), sum_sb => sum_sb(i), stats => stats(i));
  end generate;

  chk : process
    variable total : natural;
  begin
    wait until rst = '0';
    wait until dones = "11" for 5 ms;
    assert dones = "11" report "requesters did not finish" severity failure;
    for k in 1 to 4 loop
      wait until rising_edge(clk);
    end loop;
    for i in 0 to NSYS - 1 loop
      assert errs(i) = 0 report "scoreboard errors in system " & integer'image(i) severity failure;
      assert sum_rq(i) = sum_sb(i) report "checksum differs in system " & integer'image(i)
        severity failure;
      total := to_integer(stats(i).hits) + to_integer(stats(i).misses);
      assert total = OPS(i)
        report "hits + misses = " & integer'image(total) & " want " & integer'image(OPS(i))
        severity failure;
      assert stats(i).misses = stats(i).refills report "refills differ from misses" severity failure;
      assert stats(i).wbs <= stats(i).refills report "more write-backs than refills" severity failure;
      assert stats(i).wbs > 0 and stats(i).hits > stats(i).misses
        report "traffic mix is not what the test expects" severity failure;
      report "system " & integer'image(i) & ": hits " & integer'image(to_integer(stats(i).hits))
        & " misses " & integer'image(to_integer(stats(i).misses))
        & " write-backs " & integer'image(to_integer(stats(i).wbs)) severity note;
    end loop;
    report "PASS" severity note;
    fin <= true;
    wait;
  end process;
end architecture;
