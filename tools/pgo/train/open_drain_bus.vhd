-- pgo: top=open_drain_bus_tb std=2008 stop=6ms
-- Reproduces a two-wire serial bus with open-drain lines: a master with a record-based state
-- machine, two slave models (one stretching the clock), weak pull-ups on resolved std_logic
-- lines, start/stop/ack handling, many transactions, and an X pulse on an idle line.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package bus_types is
  type byte_arr_t is array (0 to 15) of std_logic_vector(7 downto 0);
  type byte_vec_t is array (natural range <>) of std_logic_vector(7 downto 0);
  type cmd_op_t is (op_start, op_write, op_read_ack, op_read_nack, op_stop);
  type mstate_t is (s_idle, s_st0, s_st1, s_st2,
                    s_wr_set, s_wr_rel, s_wr_wait, s_wr_fall,
                    s_ack_set, s_ack_rel, s_ack_wait, s_ack_fall,
                    s_rd_set, s_rd_rel, s_rd_wait, s_rd_fall,
                    s_rack_set, s_rack_rel, s_rack_wait, s_rack_fall, s_rack_free,
                    s_sp0, s_sp1, s_sp2, s_fin);
  type master_t is record
    state : mstate_t;
    op : cmd_op_t;
    bit_idx : natural range 0 to 8;
    shreg : std_logic_vector(7 downto 0);
    rx : std_logic_vector(7 downto 0);
    ack_bit : std_logic;
    scl_low : std_logic;
    sda_low : std_logic;
    stretch_ticks : natural;
  end record;
  constant master_init : master_t :=
    (s_idle, op_stop, 0, x"00", x"00", '1', '0', '0', 0);
  function init_mem (seed : natural) return byte_arr_t;
  function next_rand (x : natural) return natural;
end package bus_types;

package body bus_types is
  function init_mem (seed : natural) return byte_arr_t is
    variable r : byte_arr_t;
  begin
    for i in r'range loop
      r(i) := std_logic_vector(to_unsigned((seed + i * 29) mod 256, 8));
    end loop;
    return r;
  end function;

  -- Small multiplicative generator, the state stays below 65537.
  function next_rand (x : natural) return natural is
  begin
    return (x * 75 + 74) mod 65537;
  end function;
end package body bus_types;

library ieee;
use ieee.std_logic_1164.all;
use work.bus_types.all;

-- Master: one command at a time, bit level state machine ticking every tick_div clocks.
entity tw_master is
  generic (tick_div : positive := 5);
  port (
    clk : in std_logic;
    scl : inout std_logic;
    sda : inout std_logic;
    cmd_valid : in std_logic;
    cmd_op : in cmd_op_t;
    cmd_data : in std_logic_vector(7 downto 0);
    busy : out std_logic;
    rx_data : out std_logic_vector(7 downto 0);
    ack_bit : out std_logic;
    stretch_seen : out natural
  );
end entity tw_master;

architecture fsm of tw_master is
  signal m, m_next : master_t := master_init;
  signal tick_cnt : natural range 0 to tick_div - 1 := 0;
  signal tick : std_logic;
begin
  tick <= '1' when tick_cnt = tick_div - 1 else '0';

  regs : process (clk)
  begin
    if rising_edge(clk) then
      if tick_cnt = tick_div - 1 then
        tick_cnt <= 0;
      else
        tick_cnt <= tick_cnt + 1;
      end if;
      m <= m_next;
    end if;
  end process;

  comb : process (all)
    variable n : master_t;
    variable scl_in, sda_in : std_ulogic;
  begin
    n := m;
    scl_in := to_x01(scl);
    sda_in := to_x01(sda);
    if tick = '1' then
      case m.state is
        when s_idle =>
          if cmd_valid = '1' then
            n.op := cmd_op;
            n.shreg := cmd_data;
            n.bit_idx := 0;
            case cmd_op is
              when op_start =>
                n.sda_low := '0';
                n.state := s_st0;
              when op_write => n.state := s_wr_set;
              when op_read_ack | op_read_nack => n.state := s_rd_set;
              when op_stop => n.state := s_sp0;
            end case;
          end if;
        -- start condition: release scl, wait for it, pull sda low, then pull scl low
        when s_st0 =>
          n.scl_low := '0';
          n.state := s_st1;
        when s_st1 =>
          if scl_in = '1' then
            n.sda_low := '1';
            n.state := s_st2;
          else
            n.stretch_ticks := m.stretch_ticks + 1;
          end if;
        when s_st2 =>
          n.scl_low := '1';
          n.state := s_fin;
        -- write one bit
        when s_wr_set =>
          n.scl_low := '1';
          n.sda_low := not m.shreg(7);
          n.state := s_wr_rel;
        when s_wr_rel =>
          n.scl_low := '0';
          n.state := s_wr_wait;
        when s_wr_wait =>
          if scl_in = '1' then
            n.state := s_wr_fall;
          else
            n.stretch_ticks := m.stretch_ticks + 1;
          end if;
        when s_wr_fall =>
          n.scl_low := '1';
          n.shreg := m.shreg(6 downto 0) & '0';
          n.bit_idx := m.bit_idx + 1;
          if m.bit_idx = 7 then
            n.state := s_ack_set;
          else
            n.state := s_wr_set;
          end if;
        -- sample the slave acknowledge
        when s_ack_set =>
          n.sda_low := '0';
          n.state := s_ack_rel;
        when s_ack_rel =>
          n.scl_low := '0';
          n.state := s_ack_wait;
        when s_ack_wait =>
          if scl_in = '1' then
            n.ack_bit := sda_in;
            n.state := s_ack_fall;
          else
            n.stretch_ticks := m.stretch_ticks + 1;
          end if;
        when s_ack_fall =>
          n.scl_low := '1';
          n.state := s_fin;
        -- read one bit
        when s_rd_set =>
          n.scl_low := '1';
          n.sda_low := '0';
          n.state := s_rd_rel;
        when s_rd_rel =>
          n.scl_low := '0';
          n.state := s_rd_wait;
        when s_rd_wait =>
          if scl_in = '1' then
            n.rx := m.rx(6 downto 0) & sda_in;
            n.state := s_rd_fall;
          else
            n.stretch_ticks := m.stretch_ticks + 1;
          end if;
        when s_rd_fall =>
          n.scl_low := '1';
          n.bit_idx := m.bit_idx + 1;
          if m.bit_idx = 7 then
            n.state := s_rack_set;
          else
            n.state := s_rd_set;
          end if;
        -- send ack or nack after a read byte
        when s_rack_set =>
          if m.op = op_read_ack then
            n.sda_low := '1';
          else
            n.sda_low := '0';
          end if;
          n.state := s_rack_rel;
        when s_rack_rel =>
          n.scl_low := '0';
          n.state := s_rack_wait;
        when s_rack_wait =>
          if scl_in = '1' then
            n.state := s_rack_fall;
          else
            n.stretch_ticks := m.stretch_ticks + 1;
          end if;
        when s_rack_fall =>
          n.scl_low := '1';
          n.state := s_rack_free;
        when s_rack_free =>
          n.sda_low := '0';
          n.state := s_fin;
        -- stop condition: sda low while scl low, release scl, then release sda
        when s_sp0 =>
          n.scl_low := '1';
          n.sda_low := '1';
          n.state := s_sp1;
        when s_sp1 =>
          n.scl_low := '0';
          n.state := s_sp2;
        when s_sp2 =>
          if scl_in = '1' then
            n.sda_low := '0';
            n.state := s_fin;
          else
            n.stretch_ticks := m.stretch_ticks + 1;
          end if;
        when s_fin =>
          n.state := s_idle;
      end case;
    end if;
    m_next <= n;
  end process;

  scl <= '0' when m.scl_low = '1' else 'Z';
  sda <= '0' when m.sda_low = '1' else 'Z';
  busy <= '1' when m.state /= s_idle else '0';
  rx_data <= m.rx;
  ack_bit <= m.ack_bit;
  stretch_seen <= m.stretch_ticks;
end architecture fsm;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.bus_types.all;

-- Slave with a 16 byte memory. Optionally holds scl low for a while before read data.
entity tw_slave is
  generic (
    address : std_logic_vector(6 downto 0);
    stretch_time : time := 0 ns;
    seed : natural := 0
  );
  port (
    scl : inout std_logic;
    sda : inout std_logic;
    mem_view : out byte_arr_t
  );
end entity tw_slave;

architecture beh of tw_slave is
  constant t_out : time := 10 ns;
  signal sda_low, scl_low : boolean := false;
  signal mem_s : byte_arr_t := init_mem(seed);
begin
  sda <= '0' after t_out when sda_low else 'Z' after t_out;
  scl <= '0' when scl_low else 'Z';
  mem_view <= mem_s;

  main : process
    type st_t is (st_idle, st_addr, st_addr_ack, st_reg, st_reg_ack, st_wdata, st_wdata_ack,
                  st_rdata, st_rdata_ack, st_ignore);
    variable st : st_t := st_idle;
    variable s, d, ps, pd : std_ulogic := '1';
    variable bit_cnt : natural := 0;
    variable shift : std_logic_vector(7 downto 0) := (others => '0');
    variable ptr : natural := 0;
    variable is_read, master_ack : boolean := false;
    variable mem : byte_arr_t := init_mem(seed);
  begin
    wait on scl, sda;
    s := to_x01(scl);
    d := to_x01(sda);
    if ps = '1' and s = '1' and pd = '1' and d = '0' then
      -- start condition
      st := st_addr;
      bit_cnt := 0;
      shift := (others => '0');
      sda_low <= false;
    elsif ps = '1' and s = '1' and pd = '0' and d = '1' then
      -- stop condition
      st := st_idle;
      sda_low <= false;
    elsif ps = '0' and s = '1' then
      -- clock rising: sample
      case st is
        when st_addr | st_reg | st_wdata =>
          shift := shift(6 downto 0) & d;
          bit_cnt := bit_cnt + 1;
        when st_rdata_ack =>
          master_ack := d = '0';
        when others => null;
      end case;
    elsif ps = '1' and s = '0' then
      -- clock falling: change outputs
      case st is
        when st_addr =>
          if bit_cnt = 8 then
            if shift(7 downto 1) = address then
              is_read := shift(0) = '1';
              sda_low <= true;
              st := st_addr_ack;
            else
              st := st_ignore;
            end if;
          end if;
        when st_addr_ack =>
          bit_cnt := 0;
          if is_read then
            st := st_rdata;
            shift := mem(ptr mod 16);
            sda_low <= shift(7) = '0';
            if stretch_time > 0 ns then
              scl_low <= true;
              scl_low <= transport false after stretch_time;
            end if;
          else
            sda_low <= false;
            st := st_reg;
          end if;
        when st_reg =>
          if bit_cnt = 8 then
            ptr := to_integer(unsigned(shift)) mod 16;
            sda_low <= true;
            st := st_reg_ack;
          end if;
        when st_reg_ack =>
          bit_cnt := 0;
          sda_low <= false;
          st := st_wdata;
        when st_wdata =>
          if bit_cnt = 8 then
            mem(ptr mod 16) := shift;
            mem_s <= mem;
            ptr := ptr + 1;
            sda_low <= true;
            st := st_wdata_ack;
          end if;
        when st_wdata_ack =>
          bit_cnt := 0;
          sda_low <= false;
          st := st_wdata;
        when st_rdata =>
          bit_cnt := bit_cnt + 1;
          if bit_cnt = 8 then
            sda_low <= false;
            st := st_rdata_ack;
          else
            shift := shift(6 downto 0) & '0';
            sda_low <= shift(7) = '0';
          end if;
        when st_rdata_ack =>
          if master_ack then
            ptr := ptr + 1;
            shift := mem(ptr mod 16);
            bit_cnt := 0;
            sda_low <= shift(7) = '0';
            st := st_rdata;
            if stretch_time > 0 ns then
              scl_low <= true;
              scl_low <= transport false after stretch_time;
            end if;
          else
            sda_low <= false;
            st := st_ignore;
          end if;
        when others => null;
      end case;
    end if;
    ps := s;
    pd := d;
  end process;
end architecture beh;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.bus_types.all;

entity open_drain_bus_tb is
end entity open_drain_bus_tb;

architecture tb of open_drain_bus_tb is
  constant addr_a : std_logic_vector(6 downto 0) := "1010000";
  constant addr_b : std_logic_vector(6 downto 0) := "1101000";
  constant n_random : natural := 100;

  signal clk : std_logic := '0';
  signal finished : boolean := false;
  signal scl, sda : std_logic;
  signal cmd_valid : std_logic := '0';
  signal cmd_op : cmd_op_t := op_stop;
  signal cmd_data : std_logic_vector(7 downto 0) := (others => '0');
  signal busy : std_logic;
  signal rx_data : std_logic_vector(7 downto 0);
  signal ack_bit : std_logic;
  signal stretch_seen : natural;
  signal mem_a, mem_b : byte_arr_t;
  signal inject : boolean := false;
  signal starts, stops, x_events : natural := 0;
  signal min_period : time := time'high;
begin
  clk <= not clk after 5 ns when not finished else clk;

  -- Weak pull-ups.
  scl <= 'H';
  sda <= 'H';

  master : entity work.tw_master
    port map (clk => clk, scl => scl, sda => sda, cmd_valid => cmd_valid, cmd_op => cmd_op,
              cmd_data => cmd_data, busy => busy, rx_data => rx_data, ack_bit => ack_bit,
              stretch_seen => stretch_seen);

  slave_a : entity work.tw_slave
    generic map (address => addr_a, stretch_time => 400 ns, seed => 3)
    port map (scl => scl, sda => sda, mem_view => mem_a);

  slave_b : entity work.tw_slave
    generic map (address => addr_b, seed => 100)
    port map (scl => scl, sda => sda, mem_view => mem_b);

  -- A third driver that corrupts sda for a moment while the bus is idle.
  injector : process
  begin
    sda <= 'Z';
    wait until inject;
    sda <= 'X';
    wait for 80 ns;
    sda <= 'Z';
    wait;
  end process;

  -- Bus monitor: counts conditions, X on the lines and the shortest clock period.
  monitor : process
    variable ps, pd : std_ulogic := '1';
    variable s, d : std_ulogic;
    variable last_rise : time := 0 ns;
    variable n_start, n_stop, n_x : natural := 0;
    variable mp : time := time'high;
  begin
    wait on scl, sda;
    s := to_x01(scl);
    d := to_x01(sda);
    if now > 1 ns and (s = 'X' or d = 'X') and not (ps = 'X' or pd = 'X') then
      n_x := n_x + 1;
    end if;
    if ps = '1' and s = '1' then
      if pd = '1' and d = '0' then
        n_start := n_start + 1;
      elsif pd = '0' and d = '1' then
        n_stop := n_stop + 1;
      end if;
    end if;
    if ps = '0' and s = '1' then
      if last_rise > 0 ns and now - last_rise < mp then
        mp := now - last_rise;
      end if;
      last_rise := now;
    end if;
    ps := s;
    pd := d;
    starts <= n_start;
    stops <= n_stop;
    x_events <= n_x;
    min_period <= mp;
  end process;

  stim : process
    variable seed : natural := 4242;
    variable model_a : byte_arr_t := init_mem(3);
    variable model_b : byte_arr_t := init_mem(100);
    variable n_cmds_start : natural := 0;
    variable n_cmds_stop : natural := 0;
    variable n_nack : natural := 0;
    variable ack : std_logic;
    variable got : byte_vec_t(0 to 7);
    variable data : byte_vec_t(0 to 7);
    variable addr : std_logic_vector(6 downto 0);
    variable reg, cnt : natural;
    variable use_a : boolean;
    variable total_bytes : natural := 0;
    variable stretch_before : natural;

    procedure issue (op : cmd_op_t; d : std_logic_vector(7 downto 0) := x"00") is
    begin
      cmd_op <= op;
      cmd_data <= d;
      cmd_valid <= '1';
      wait until busy = '1' for 10 us;
      assert busy = '1' report "master did not accept command" severity failure;
      cmd_valid <= '0';
      wait until busy = '0' for 200 us;
      assert busy = '0' report "master stuck busy" severity failure;
      case op is
        when op_start => n_cmds_start := n_cmds_start + 1;
        when op_stop => n_cmds_stop := n_cmds_stop + 1;
        when others => null;
      end case;
    end procedure;

    procedure begin_transfer (a : std_logic_vector(6 downto 0); rw : std_logic; ack_out : out std_logic) is
    begin
      issue(op_start);
      issue(op_write, a & rw);
      ack_out := ack_bit;
    end procedure;

    procedure write_regs (a : std_logic_vector(6 downto 0); r : natural; d : byte_vec_t; ok : out boolean) is
      variable ack : std_logic;
    begin
      ok := true;
      begin_transfer(a, '0', ack);
      if to_x01(ack) /= '0' then
        ok := false;
      else
        issue(op_write, std_logic_vector(to_unsigned(r, 8)));
        ok := ok and to_x01(ack_bit) = '0';
        for i in d'range loop
          issue(op_write, d(i));
          ok := ok and to_x01(ack_bit) = '0';
        end loop;
      end if;
      issue(op_stop);
    end procedure;

    procedure read_regs (a : std_logic_vector(6 downto 0); r : natural; n : positive; d : out byte_vec_t) is
      variable ack : std_logic;
    begin
      begin_transfer(a, '0', ack);
      assert to_x01(ack) = '0' report "read: address nack" severity failure;
      issue(op_write, std_logic_vector(to_unsigned(r, 8)));
      begin_transfer(a, '1', ack);
      assert to_x01(ack) = '0' report "read: address nack after restart" severity failure;
      for i in 0 to n - 1 loop
        if i = n - 1 then
          issue(op_read_nack);
        else
          issue(op_read_ack);
        end if;
        d(d'left + i) := rx_data;
      end loop;
      issue(op_stop);
    end procedure;

    procedure update_model (use_a : boolean; r : natural; d : byte_vec_t) is
    begin
      for i in d'range loop
        if use_a then
          model_a((r + i - d'left) mod 16) := d(i);
        else
          model_b((r + i - d'left) mod 16) := d(i);
        end if;
      end loop;
    end procedure;

    variable ok : boolean;
  begin
    wait for 200 ns;
    assert to_x01(scl) = '1' and to_x01(sda) = '1' report "bus not idle high" severity failure;
    assert scl = 'H' and sda = 'H' report "pull-up value lost" severity failure;

    -- Directed: write three bytes to each slave and read them back.
    data(0 to 2) := (x"11", x"22", x"33");
    write_regs(addr_a, 3, data(0 to 2), ok);
    assert ok report "directed write A not acknowledged" severity failure;
    update_model(true, 3, data(0 to 2));
    read_regs(addr_a, 3, 3, got(0 to 2));
    assert got(0 to 2) = data(0 to 2) report "directed read A mismatch" severity failure;
    assert stretch_seen > 0 report "slave A did not stretch the clock" severity failure;
    data(0 to 2) := (x"a5", x"5a", x"ff");
    write_regs(addr_b, 12, data(0 to 2), ok);
    assert ok report "directed write B not acknowledged" severity failure;
    update_model(false, 12, data(0 to 2));
    read_regs(addr_b, 12, 3, got(0 to 2));
    assert got(0 to 2) = data(0 to 2) report "directed read B mismatch" severity failure;
    assert mem_a = model_a and mem_b = model_b report "memory differs after directed phase" severity failure;

    -- A device that is not there: the address byte is not acknowledged.
    write_regs("0110011", 0, data(0 to 0), ok);
    assert not ok report "missing device acknowledged" severity failure;
    n_nack := n_nack + 1;

    -- Corrupt sda for a moment while idle; the transfers after it must still work.
    inject <= true;
    wait for 300 ns;
    assert x_events = 1 report "X pulse not seen once, saw " & integer'image(x_events) severity failure;
    assert to_x01(sda) = '1' report "sda did not recover" severity failure;

    -- Random mix of writes, read-backs, wrap-around and missing devices.
    for t in 1 to n_random loop
      seed := next_rand(seed);
      use_a := seed mod 2 = 0;
      seed := next_rand(seed);
      reg := seed mod 16;
      seed := next_rand(seed);
      cnt := seed mod 4 + 1;
      for i in 0 to cnt - 1 loop
        seed := next_rand(seed);
        data(i) := std_logic_vector(to_unsigned(seed mod 256, 8));
      end loop;
      if use_a then
        addr := addr_a;
      else
        addr := addr_b;
      end if;
      if t mod 17 = 0 then
        addr := std_logic_vector(unsigned(addr) + 1);
        write_regs(addr, reg, data(0 to cnt - 1), ok);
        assert not ok report "wrong address acknowledged" severity failure;
        n_nack := n_nack + 1;
      else
        write_regs(addr, reg, data(0 to cnt - 1), ok);
        assert ok report "random write not acknowledged" severity failure;
        update_model(use_a, reg, data(0 to cnt - 1));
        stretch_before := stretch_seen;
        read_regs(addr, reg, cnt, got(0 to cnt - 1));
        for i in 0 to cnt - 1 loop
          if use_a then
            assert got(i) = model_a((reg + i) mod 16) report "read mismatch on A" severity failure;
          else
            assert got(i) = model_b((reg + i) mod 16) report "read mismatch on B" severity failure;
          end if;
        end loop;
        if use_a then
          assert stretch_seen > stretch_before report "no stretch on read from A" severity failure;
        end if;
        total_bytes := total_bytes + 2 * cnt;
      end if;
    end loop;

    wait for 300 ns;
    assert mem_a = model_a report "slave A memory differs" severity failure;
    assert mem_b = model_b report "slave B memory differs" severity failure;
    assert stops = n_cmds_stop report "stop count " & integer'image(stops) & " /= " & integer'image(n_cmds_stop) severity failure;
    assert starts = n_cmds_start report "start count " & integer'image(starts) & " /= " & integer'image(n_cmds_start) severity failure;
    assert min_period >= 190 ns report "clock too fast: " & time'image(min_period) severity failure;
    assert scl = 'H' and sda = 'H' report "bus not released at the end" severity failure;
    report "transactions: " & integer'image(n_cmds_stop) & ", bytes: " & integer'image(total_bytes) &
           ", nacks: " & integer'image(n_nack) severity note;
    report "PASS" severity note;
    finished <= true;
    wait;
  end process;
end architecture tb;
