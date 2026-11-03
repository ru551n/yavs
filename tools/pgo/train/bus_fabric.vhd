-- pgo: top=bus_fabric_tb std=1993 stop=700 us relaxed
-- Reproduces a VHDL-93 bus system: round-robin arbiter, address decoding fabric with
-- record bus ports and record arrays, generate loops of masters, RAM, timer, UART
-- and interrupt slaves, written with std_logic_arith and std_logic_unsigned.

library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;

package bf_pkg is
  subtype addr_t is std_logic_vector(15 downto 0);
  subtype data_t is std_logic_vector(31 downto 0);

  type bus_m2s_t is record
    req   : std_logic;
    we    : std_logic;
    addr  : addr_t;
    wdata : data_t;
  end record;

  type bus_s2m_t is record
    ack   : std_logic;
    err   : std_logic;
    rdata : data_t;
  end record;

  type m2s_array_t is array (natural range <>) of bus_m2s_t;
  type s2m_array_t is array (natural range <>) of bus_s2m_t;
  type mem512_t is array (0 to 511) of data_t;
  type mem2k_t is array (0 to 2047) of data_t;
  type int_array_t is array (natural range <>) of integer;

  constant M2S_IDLE : bus_m2s_t := ('0', '0', (others => '0'), (others => '0'));
  constant S2M_IDLE : bus_s2m_t := ('0', '0', (others => '0'));
  constant ERR_DATA : data_t := x"DEADBEEF";

  function decode_slave(a : addr_t) return integer;
  function gate(b : bus_m2s_t; en : boolean) return bus_m2s_t;
  function lfsr_next(x : data_t) return data_t;
  function zx(v : std_logic_vector; n : integer) return std_logic_vector;
  function ram_addr(region, priv, idx : integer) return addr_t;
  function to_hex(v : std_logic_vector) return string;

  procedure bus_xfer(signal clk : in std_logic;
                     signal mo : out bus_m2s_t;
                     signal mi : in bus_s2m_t;
                     constant w : in std_logic;
                     constant a : in addr_t;
                     constant d : in data_t;
                     variable rdata : out data_t;
                     variable err : out boolean);
  procedure bus_wr(signal clk : in std_logic;
                   signal mo : out bus_m2s_t;
                   signal mi : in bus_s2m_t;
                   constant a : in addr_t;
                   constant d : in data_t;
                   variable err : out boolean);
  procedure bus_rd(signal clk : in std_logic;
                   signal mo : out bus_m2s_t;
                   signal mi : in bus_s2m_t;
                   constant a : in addr_t;
                   variable rdata : out data_t;
                   variable err : out boolean);
  procedure bump(variable counter : inout integer; constant by : in integer);
end bf_pkg;

package body bf_pkg is
  -- Address map by the top three bits: 0 and 1 RAM A, 2 RAM B, 3 timer, 4 UART, 5 interrupts.
  function decode_slave(a : addr_t) return integer is
  begin
    case conv_integer(a(15 downto 13)) is
      when 0 to 1 => return 0;
      when 2      => return 1;
      when 3      => return 2;
      when 4      => return 3;
      when 5      => return 4;
      when others => return -1;
    end case;
  end decode_slave;

  function gate(b : bus_m2s_t; en : boolean) return bus_m2s_t is
    variable r : bus_m2s_t := b;
  begin
    if not en then
      r.req := '0';
    end if;
    return r;
  end gate;

  -- Eight steps of a 32 bit Fibonacci shift register.
  function lfsr_next(x : data_t) return data_t is
    variable r : data_t := x;
    variable fb : std_logic;
  begin
    for i in 1 to 8 loop
      fb := r(31) xor r(21) xor r(1) xor r(0);
      r := r(30 downto 0) & fb;
    end loop;
    return r;
  end lfsr_next;

  function zx(v : std_logic_vector; n : integer) return std_logic_vector is
    variable r : std_logic_vector(n - 1 downto 0) := (others => '0');
  begin
    r(v'length - 1 downto 0) := v;
    return r;
  end zx;

  function ram_addr(region, priv, idx : integer) return addr_t is
    variable a : addr_t := (others => '0');
  begin
    a(15 downto 13) := conv_std_logic_vector(region, 3);
    a(8 downto 0) := conv_std_logic_vector(idx, 9);
    if priv = 1 then
      a(9) := '1';
    end if;
    return a;
  end ram_addr;

  function to_hex(v : std_logic_vector) return string is
    constant digits : string(1 to 16) := "0123456789ABCDEF";
    constant n : integer := (v'length + 3) / 4;
    variable padded : std_logic_vector(4 * n - 1 downto 0) := (others => '0');
    variable nib : std_logic_vector(3 downto 0);
    variable s : string(1 to n);
  begin
    padded(v'length - 1 downto 0) := v;
    for i in 1 to n loop
      nib := padded(4 * (n - i) + 3 downto 4 * (n - i));
      if is_x(nib) then
        s(i) := 'X';
      else
        s(i) := digits(conv_integer(nib) + 1);
      end if;
    end loop;
    return s;
  end to_hex;

  -- One bus transfer: hold the request until ack or err arrives at a clock edge.
  procedure bus_xfer(signal clk : in std_logic;
                     signal mo : out bus_m2s_t;
                     signal mi : in bus_s2m_t;
                     constant w : in std_logic;
                     constant a : in addr_t;
                     constant d : in data_t;
                     variable rdata : out data_t;
                     variable err : out boolean) is
  begin
    mo <= ('1', w, a, d);
    wait until rising_edge(clk) and (mi.ack = '1' or mi.err = '1');
    rdata := mi.rdata;
    err := (mi.err = '1');
    mo.req <= '0';
  end bus_xfer;

  procedure bus_wr(signal clk : in std_logic;
                   signal mo : out bus_m2s_t;
                   signal mi : in bus_s2m_t;
                   constant a : in addr_t;
                   constant d : in data_t;
                   variable err : out boolean) is
    variable dummy : data_t;
  begin
    bus_xfer(clk, mo, mi, '1', a, d, dummy, err);
  end bus_wr;

  procedure bus_rd(signal clk : in std_logic;
                   signal mo : out bus_m2s_t;
                   signal mi : in bus_s2m_t;
                   constant a : in addr_t;
                   variable rdata : out data_t;
                   variable err : out boolean) is
  begin
    bus_xfer(clk, mo, mi, '0', a, (others => '0'), rdata, err);
  end bus_rd;

  procedure bump(variable counter : inout integer; constant by : in integer) is
  begin
    counter := counter + by;
  end bump;
end bf_pkg;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

-- RAM slave with a configurable number of wait states.
entity bf_ram is
  generic (
    g_mode : integer := 0;
    g_wait : integer := 0
  );
  port (
    clk, rst : in  std_logic;
    s_i      : in  bus_m2s_t;
    s_o      : out bus_s2m_t
  );
end bf_ram;

architecture rtl of bf_ram is
  signal ack_q : std_logic := '0';
  signal rd_q  : data_t := (others => '0');
begin
  s_o <= (ack => ack_q, err => '0', rdata => rd_q);

  access_p : process (clk)
    variable mem : mem2k_t := (others => (others => '0'));
    variable cnt : integer range 0 to 7 := 0;
    variable ix  : std_logic_vector(10 downto 0);
  begin
    if rising_edge(clk) then
      ack_q <= '0';
      if rst = '1' then
        cnt := 0;
      elsif s_i.req = '1' and ack_q = '0' then
        if cnt >= g_wait then
          cnt := 0;
          if g_mode = 0 then
            ix := s_i.addr(13) & s_i.addr(9 downto 0);
          else
            ix := '0' & s_i.addr(9 downto 0);
          end if;
          if s_i.we = '1' then
            mem(conv_integer(ix)) := s_i.wdata;
          else
            rd_q <= mem(conv_integer(ix));
          end if;
          ack_q <= '1';
        else
          cnt := cnt + 1;
        end if;
      end if;
    end if;
  end process;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

-- Timer: 0 control (enable, irq enable, reload), 1 count, 2 compare, 3 status (write 1 clears).
entity bf_timer is
  port (
    clk, rst : in  std_logic;
    s_i      : in  bus_m2s_t;
    s_o      : out bus_s2m_t;
    irq      : out std_logic
  );
end bf_timer;

architecture rtl of bf_timer is
  signal ctrl    : std_logic_vector(2 downto 0) := "000";
  signal count   : data_t := (others => '0');
  signal compare : data_t := (others => '0');
  signal pre     : std_logic_vector(1 downto 0) := "00";
  signal flag    : std_logic := '0';
  signal ack_q   : std_logic := '0';
  signal rd_q    : data_t := (others => '0');
begin
  s_o <= (ack => ack_q, err => '0', rdata => rd_q);
  irq <= flag and ctrl(1);

  regs : process (clk)
    variable off : integer range 0 to 3;
    variable rd  : data_t;
  begin
    if rising_edge(clk) then
      ack_q <= '0';
      if rst = '1' then
        ctrl <= "000";
        count <= (others => '0');
        compare <= (others => '0');
        pre <= "00";
        flag <= '0';
      else
        if ctrl(0) = '1' then
          pre <= pre + 1;
          if pre = "11" then
            count <= count + 1;
            if (count + 1) = compare then
              flag <= '1';
              if ctrl(2) = '1' then
                count <= (others => '0');
              else
                ctrl(0) <= '0';
              end if;
            end if;
          end if;
        end if;
        if s_i.req = '1' and ack_q = '0' then
          off := conv_integer(s_i.addr(1 downto 0));
          ack_q <= '1';
          if s_i.we = '1' then
            case off is
              when 0      => ctrl <= s_i.wdata(2 downto 0);
              when 1      => count <= s_i.wdata;
              when 2      => compare <= s_i.wdata;
              when others =>
                if s_i.wdata(0) = '1' then
                  flag <= '0';
                end if;
            end case;
          else
            rd := (others => '0');
            case off is
              when 0      => rd(2 downto 0) := ctrl;
              when 1      => rd := count;
              when 2      => rd := compare;
              when others => rd(0) := flag;
            end case;
            rd_q <= rd;
          end if;
        end if;
      end if;
    end if;
  end process;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;

-- UART transmitter: start bit, eight data bits LSB first, stop bit; div clocks per bit.
entity bf_uart_tx is
  port (
    clk, rst : in  std_logic;
    div      : in  std_logic_vector(7 downto 0);
    start    : in  std_logic;
    data     : in  std_logic_vector(7 downto 0);
    txd      : out std_logic;
    busy     : out std_logic
  );
end bf_uart_tx;

architecture rtl of bf_uart_tx is
  type state_t is (ST_IDLE, ST_START, ST_DATA, ST_STOP);
  signal state : state_t := ST_IDLE;
  signal cnt   : std_logic_vector(7 downto 0) := (others => '0');
  signal sh    : std_logic_vector(7 downto 0) := (others => '0');
  signal nbit  : integer range 0 to 7 := 0;
  signal line  : std_logic := '1';
begin
  txd <= line;
  busy <= '0' when state = ST_IDLE else '1';

  fsm : process (clk)
    variable bit_end : boolean;
  begin
    if rising_edge(clk) then
      bit_end := (cnt = div - 1);
      if rst = '1' then
        state <= ST_IDLE;
        line <= '1';
        cnt <= (others => '0');
      else
        case state is
          when ST_IDLE =>
            if start = '1' then
              sh <= data;
              line <= '0';
              cnt <= (others => '0');
              state <= ST_START;
            end if;
          when ST_START =>
            if bit_end then
              cnt <= (others => '0');
              line <= sh(0);
              sh <= '0' & sh(7 downto 1);
              nbit <= 0;
              state <= ST_DATA;
            else
              cnt <= cnt + 1;
            end if;
          when ST_DATA =>
            if bit_end then
              cnt <= (others => '0');
              if nbit = 7 then
                line <= '1';
                state <= ST_STOP;
              else
                line <= sh(0);
                sh <= '0' & sh(7 downto 1);
                nbit <= nbit + 1;
              end if;
            else
              cnt <= cnt + 1;
            end if;
          when ST_STOP =>
            if bit_end then
              state <= ST_IDLE;
            else
              cnt <= cnt + 1;
            end if;
        end case;
      end if;
    end if;
  end process;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;

-- UART receiver: finds the start edge, samples each bit in the middle, pulses valid.
entity bf_uart_rx is
  port (
    clk, rst : in  std_logic;
    div      : in  std_logic_vector(7 downto 0);
    rxd      : in  std_logic;
    data     : out std_logic_vector(7 downto 0);
    valid    : out std_logic
  );
end bf_uart_rx;

architecture rtl of bf_uart_rx is
  type state_t is (RX_IDLE, RX_START, RX_DATA, RX_STOP);
  signal state : state_t := RX_IDLE;
  signal s1, s2 : std_logic := '1';
  signal cnt   : std_logic_vector(7 downto 0) := (others => '0');
  signal sh    : std_logic_vector(7 downto 0) := (others => '0');
  signal nbit  : integer range 0 to 7 := 0;
  signal dout  : std_logic_vector(7 downto 0) := (others => '0');
  signal pulse : std_logic := '0';
  alias  half  : std_logic_vector(6 downto 0) is div(7 downto 1);
begin
  data <= dout;
  valid <= pulse;

  fsm : process (clk)
  begin
    if rising_edge(clk) then
      s1 <= rxd;
      s2 <= s1;
      pulse <= '0';
      if rst = '1' then
        state <= RX_IDLE;
      else
        case state is
          when RX_IDLE =>
            if s2 = '0' then
              cnt <= (others => '0');
              state <= RX_START;
            end if;
          when RX_START =>
            if cnt = conv_std_logic_vector(conv_integer(half), 8) then
              cnt <= (others => '0');
              nbit <= 0;
              if s2 = '0' then
                state <= RX_DATA;
              else
                state <= RX_IDLE;
              end if;
            else
              cnt <= cnt + 1;
            end if;
          when RX_DATA =>
            if cnt = div - 1 then
              cnt <= (others => '0');
              sh <= s2 & sh(7 downto 1);
              if nbit = 7 then
                state <= RX_STOP;
              else
                nbit <= nbit + 1;
              end if;
            else
              cnt <= cnt + 1;
            end if;
          when RX_STOP =>
            if cnt = div - 1 then
              dout <= sh;
              pulse <= '1';
              state <= RX_IDLE;
            else
              cnt <= cnt + 1;
            end if;
        end case;
      end if;
    end if;
  end process;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

-- UART registers: 0 tx data, 1 rx data (read clears), 2 status (busy, rx valid, tx done), 3 divider.
entity bf_uart is
  port (
    clk, rst : in  std_logic;
    s_i      : in  bus_m2s_t;
    s_o      : out bus_s2m_t;
    txd      : out std_logic;
    rxd      : in  std_logic;
    irq_rx   : out std_logic;
    irq_tx   : out std_logic
  );
end bf_uart;

architecture rtl of bf_uart is
  component bf_uart_rx
    port (
      clk, rst : in  std_logic;
      div      : in  std_logic_vector(7 downto 0);
      rxd      : in  std_logic;
      data     : out std_logic_vector(7 downto 0);
      valid    : out std_logic
    );
  end component;

  signal div      : std_logic_vector(7 downto 0) := conv_std_logic_vector(10, 8);
  signal start    : std_logic := '0';
  signal tx_data  : std_logic_vector(7 downto 0) := (others => '0');
  signal tx_busy  : std_logic;
  signal busy_d   : std_logic := '0';
  signal tx_done  : std_logic := '0';
  signal rx_byte  : std_logic_vector(7 downto 0);
  signal rx_pulse : std_logic;
  signal rx_valid : std_logic := '0';
  signal rx_hold  : std_logic_vector(7 downto 0) := (others => '0');
  signal ack_q    : std_logic := '0';
  signal rd_q     : data_t := (others => '0');
begin
  s_o <= (ack => ack_q, err => '0', rdata => rd_q);
  irq_rx <= rx_valid;
  irq_tx <= tx_done;

  u_tx : entity work.bf_uart_tx
    port map (clk => clk, rst => rst, div => div, start => start, data => tx_data,
              txd => txd, busy => tx_busy);

  u_rx : bf_uart_rx
    port map (clk => clk, rst => rst, div => div, rxd => rxd, data => rx_byte, valid => rx_pulse);

  regs : process (clk)
    variable off : integer range 0 to 3;
    variable rd  : data_t;
  begin
    if rising_edge(clk) then
      ack_q <= '0';
      start <= '0';
      busy_d <= tx_busy;
      if busy_d = '1' and tx_busy = '0' then
        tx_done <= '1';
      end if;
      if rst = '1' then
        rx_valid <= '0';
        tx_done <= '0';
      else
        if s_i.req = '1' and ack_q = '0' then
          off := conv_integer(s_i.addr(1 downto 0));
          ack_q <= '1';
          if s_i.we = '1' then
            case off is
              when 0 =>
                tx_data <= s_i.wdata(7 downto 0);
                start <= '1';
              when 2 =>
                if s_i.wdata(2) = '1' then
                  tx_done <= '0';
                end if;
              when 3 =>
                div <= s_i.wdata(7 downto 0);
              when others => null;
            end case;
          else
            rd := (others => '0');
            case off is
              when 1 =>
                rd := zx(rx_hold, 32);
                rx_valid <= '0';
              when 2 =>
                rd(0) := tx_busy;
                rd(1) := rx_valid;
                rd(2) := tx_done;
              when 3 =>
                rd := zx(div, 32);
              when others => null;
            end case;
            rd_q <= rd;
          end if;
        end if;
        if rx_pulse = '1' then
          rx_hold <= rx_byte;
          rx_valid <= '1';
        end if;
      end if;
    end if;
  end process;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

-- Interrupt controller: 0 pending (read), 1 enable, 2 acknowledge (write 1 clears).
entity bf_intc is
  port (
    clk, rst : in  std_logic;
    s_i      : in  bus_m2s_t;
    s_o      : out bus_s2m_t;
    irq_in   : in  std_logic_vector(3 downto 0);
    irq_out  : out std_logic
  );
end bf_intc;

architecture rtl of bf_intc is
  signal pend  : std_logic_vector(3 downto 0) := (others => '0');
  signal en    : std_logic_vector(3 downto 0) := (others => '0');
  signal ack_q : std_logic := '0';
  signal rd_q  : data_t := (others => '0');
begin
  s_o <= (ack => ack_q, err => '0', rdata => rd_q);
  irq_out <= '1' when (pend and en) /= "0000" else '0';

  regs : process (clk)
    variable off : integer range 0 to 3;
    variable p   : std_logic_vector(3 downto 0);
    variable rd  : data_t;
  begin
    if rising_edge(clk) then
      ack_q <= '0';
      p := pend;
      if rst = '1' then
        p := (others => '0');
        en <= (others => '0');
      elsif s_i.req = '1' and ack_q = '0' then
        off := conv_integer(s_i.addr(1 downto 0));
        ack_q <= '1';
        if s_i.we = '1' then
          case off is
            when 1      => en <= s_i.wdata(3 downto 0);
            when 2      => p := p and not s_i.wdata(3 downto 0);
            when others => null;
          end case;
        else
          rd := (others => '0');
          case off is
            when 0      => rd(3 downto 0) := p;
            when 1      => rd(3 downto 0) := en;
            when others => null;
          end case;
          rd_q <= rd;
        end if;
      end if;
      if rst = '0' then
        p := p or irq_in;
      end if;
      pend <= p;
    end if;
  end process;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;

-- Round-robin arbiter: one grant at a time, released when the transfer completes.
entity bf_arbiter is
  generic (g_n : integer := 3);
  port (
    clk, rst : in  std_logic;
    req      : in  std_logic_vector(g_n - 1 downto 0);
    done     : in  std_logic;
    grant    : out std_logic_vector(g_n - 1 downto 0)
  );
end bf_arbiter;

architecture rtl of bf_arbiter is
  signal gnt : std_logic_vector(g_n - 1 downto 0) := (others => '0');
begin
  grant <= gnt;

  pick : process (clk)
    variable last : integer range 0 to g_n - 1 := 0;
    variable held : boolean := false;
    variable idx  : integer range 0 to g_n - 1;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        gnt <= (others => '0');
        held := false;
        last := 0;
      elsif held then
        if done = '1' then
          gnt <= (others => '0');
          held := false;
        end if;
      else
        for i in 1 to g_n loop
          idx := (last + i) mod g_n;
          if req(idx) = '1' then
            gnt <= (others => '0');
            gnt(idx) <= '1';
            last := idx;
            held := true;
            exit;
          end if;
        end loop;
      end if;
    end if;
  end process;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

-- Fabric: arbitrates the masters, decodes the address, routes the response back.
entity bf_fabric is
  generic (g_masters : integer := 3);
  port (
    clk, rst : in  std_logic;
    m_in     : in  m2s_array_t;
    m_out    : out s2m_array_t;
    s_out    : out m2s_array_t;
    s_in     : in  s2m_array_t
  );
end bf_fabric;

architecture rtl of bf_fabric is
  component bf_arbiter
    generic (g_n : integer := 3);
    port (
      clk, rst : in  std_logic;
      req      : in  std_logic_vector(g_n - 1 downto 0);
      done     : in  std_logic;
      grant    : out std_logic_vector(g_n - 1 downto 0)
    );
  end component;

  signal reqv  : std_logic_vector(g_masters - 1 downto 0);
  signal grant : std_logic_vector(g_masters - 1 downto 0);
  signal act   : bus_m2s_t := M2S_IDLE;
  signal sel   : integer range -1 to 15 := -1;
  signal resp  : bus_s2m_t := S2M_IDLE;
  signal done  : std_logic;
begin
  req_gen : for i in m_in'range generate
    reqv(i) <= m_in(i).req;
  end generate;

  u_arb : bf_arbiter
    generic map (g_n => g_masters)
    port map (clk => clk, rst => rst, req => reqv, done => done, grant => grant);

  select_master : process (grant, m_in)
    variable a : bus_m2s_t;
  begin
    a := M2S_IDLE;
    for i in m_in'range loop
      if grant(i) = '1' then
        a := m_in(i);
      end if;
    end loop;
    act <= a;
  end process;

  sel <= decode_slave(act.addr);

  slave_gen : for j in s_out'range generate
    s_out(j) <= gate(act, sel = j);
  end generate;

  -- An unmapped address answers with an error and a marker value.
  respond : process (act, sel, s_in)
  begin
    if act.req = '0' then
      resp <= S2M_IDLE;
    elsif sel < 0 then
      resp <= ('0', '1', ERR_DATA);
    else
      resp <= s_in(sel);
    end if;
  end process;

  done <= resp.ack or resp.err;

  master_gen : for i in m_out'range generate
    m_out(i) <= resp when grant(i) = '1' else S2M_IDLE;
  end generate;
end rtl;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

-- Stress master: random writes, reads and read-modify-writes in two private RAM windows.
entity bf_master is
  generic (
    g_id   : integer;
    g_seed : integer;
    g_ops  : integer
  );
  port (
    clk, rst : in  std_logic;
    mo       : out bus_m2s_t;
    mi       : in  bus_s2m_t;
    done     : out std_logic;
    errors   : out integer;
    ops      : out integer
  );
end bf_master;

architecture beh of bf_master is
begin
  stress : process
    variable x    : data_t := conv_std_logic_vector(g_seed, 32);
    variable sa   : mem512_t := (others => (others => '0'));
    variable sb   : mem512_t := (others => (others => '0'));
    variable idx  : integer range 0 to 511;
    variable a    : addr_t;
    variable wd   : data_t;
    variable rd   : data_t;
    variable e    : boolean;
    variable nerr : integer := 0;
    variable nops : integer := 0;
    variable csum : data_t := (others => '0');

    procedure check(got, want : in data_t; at : in addr_t) is
    begin
      if got /= want then
        bump(nerr, 1);
        report "master " & integer'image(g_id) & " read mismatch at " & to_hex(at)
          & ": got " & to_hex(got) & " want " & to_hex(want) severity error;
      end if;
    end check;
  begin
    done <= '0';
    mo <= M2S_IDLE;
    wait until rst = '0';
    wait until rising_edge(clk);
    for n in 1 to g_ops loop
      x := lfsr_next(x);
      idx := conv_integer(x(12 downto 4));
      case conv_integer(x(3 downto 0)) is
        when 0 to 4 =>
          wd := x + conv_std_logic_vector(n, 32);
          a := ram_addr(g_id - 1, 0, idx);
          bus_wr(clk, mo, mi, a, wd, e);
          sa(idx) := wd;
        when 5 to 7 =>
          a := ram_addr(g_id - 1, 0, idx);
          bus_rd(clk, mo, mi, a, rd, e);
          check(rd, sa(idx), a);
          csum := csum + rd;
        when 8 to 10 =>
          wd := x - conv_std_logic_vector(n, 32);
          a := ram_addr(2, g_id - 1, idx);
          bus_wr(clk, mo, mi, a, wd, e);
          sb(idx) := wd;
        when 11 to 13 =>
          a := ram_addr(2, g_id - 1, idx);
          bus_rd(clk, mo, mi, a, rd, e);
          check(rd, sb(idx), a);
          csum := csum + rd;
        when others =>
          a := ram_addr(g_id - 1, 0, idx);
          bus_rd(clk, mo, mi, a, rd, e);
          check(rd, sa(idx), a);
          wd := rd + 1;
          bus_wr(clk, mo, mi, a, wd, e);
          sa(idx) := wd;
      end case;
      if e then
        bump(nerr, 1);
      end if;
      nops := nops + 1;
    end loop;
    -- Final sweep over both windows.
    for i in sa'range loop
      a := ram_addr(g_id - 1, 0, i);
      bus_rd(clk, mo, mi, a, rd, e);
      check(rd, sa(i), a);
      a := ram_addr(2, g_id - 1, i);
      bus_rd(clk, mo, mi, a, rd, e);
      check(rd, sb(i), a);
      nops := nops + 2;
    end loop;
    errors <= nerr;
    ops <= nops;
    done <= '1';
    wait;
  end process;
end beh;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

-- Processor-like master: sets up the timer, interrupts and UART, sends bytes through the
-- loopback, services interrupts, then makes accesses to unmapped addresses.
entity bf_cpu is
  generic (g_bytes : integer := 32);
  port (
    clk, rst : in  std_logic;
    mo       : out bus_m2s_t;
    mi       : in  bus_s2m_t;
    irq      : in  std_logic;
    done     : out std_logic;
    errors   : out integer;
    ticks    : out integer
  );
end bf_cpu;

architecture beh of bf_cpu is
  constant A_TIMER : addr_t := x"6000";
  constant A_UART  : addr_t := x"8000";
  constant A_INTC  : addr_t := x"A000";
  subtype byte_t is std_logic_vector(7 downto 0);
  type byte_q_t is array (0 to 255) of byte_t;
begin
  prog : process
    variable rd     : data_t;
    variable e      : boolean;
    variable nerr   : integer := 0;
    variable nticks : integer := 0;
    variable nrx    : integer := 0;
    variable ntxd   : integer := 0;
    variable x      : data_t := conv_std_logic_vector(16#5EED#, 32);
    variable b      : byte_t;
    variable rxq    : byte_q_t;

    procedure rd_reg(base : in addr_t; off : in integer; v : out data_t) is
      variable ee : boolean;
    begin
      bus_rd(clk, mo, mi, base + off, v, ee);
      if ee then
        bump(nerr, 1);
      end if;
    end rd_reg;

    procedure wr_reg(base : in addr_t; off : in integer; v : in data_t) is
      variable ee : boolean;
    begin
      bus_wr(clk, mo, mi, base + off, v, ee);
      if ee then
        bump(nerr, 1);
      end if;
    end wr_reg;

    procedure idle(n : in integer) is
    begin
      for i in 1 to n loop
        wait until rising_edge(clk);
      end loop;
    end idle;

    procedure service is
      variable p : data_t;
      variable v : data_t;
    begin
      rd_reg(A_INTC, 0, p);
      if p(0) = '1' then
        wr_reg(A_TIMER, 3, conv_std_logic_vector(1, 32));
        nticks := nticks + 1;
      end if;
      if p(1) = '1' then
        rd_reg(A_UART, 1, v);
        rxq(nrx mod 256) := v(7 downto 0);
        nrx := nrx + 1;
      end if;
      if p(2) = '1' then
        wr_reg(A_UART, 2, conv_std_logic_vector(4, 32));
        ntxd := ntxd + 1;
      end if;
      wr_reg(A_INTC, 2, p);
    end service;
  begin
    done <= '0';
    mo <= M2S_IDLE;
    wait until rst = '0';
    idle(2);
    wr_reg(A_TIMER, 2, conv_std_logic_vector(150, 32));
    wr_reg(A_TIMER, 0, conv_std_logic_vector(7, 32));
    wr_reg(A_INTC, 1, conv_std_logic_vector(7, 32));
    wr_reg(A_UART, 3, conv_std_logic_vector(10, 32));
    rd_reg(A_UART, 3, rd);
    if rd /= conv_std_logic_vector(10, 32) then
      bump(nerr, 1);
    end if;

    for n in 0 to g_bytes - 1 loop
      loop
        rd_reg(A_UART, 2, rd);
        exit when rd(0) = '0';
        if irq = '1' then
          service;
        end if;
      end loop;
      x := lfsr_next(x);
      b := x(15 downto 8);
      wr_reg(A_UART, 0, zx(b, 32));
      while nrx <= n loop
        if irq = '1' then
          service;
        else
          idle(6);
        end if;
      end loop;
      if rxq(n mod 256) /= b then
        bump(nerr, 1);
        report "uart loopback byte " & integer'image(n) & " got " & to_hex(rxq(n mod 256))
          & " want " & to_hex(b) severity error;
      end if;
    end loop;

    while nticks < 3 loop
      if irq = '1' then
        service;
      else
        idle(4);
      end if;
    end loop;

    -- Unmapped addresses must answer with an error.
    bus_wr(clk, mo, mi, x"E000", x"12345678", e);
    if not e then
      bump(nerr, 1);
    end if;
    bus_rd(clk, mo, mi, x"FFFC", rd, e);
    if not e or rd /= ERR_DATA then
      bump(nerr, 1);
    end if;
    rd_reg(A_TIMER, 1, rd);
    if rd = conv_std_logic_vector(0, 32) then
      bump(nerr, 1);
    end if;
    if ntxd = 0 then
      bump(nerr, 1);
    end if;
    errors <= nerr;
    ticks <= nticks;
    done <= '1';
    wait;
  end process;
end beh;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use work.bf_pkg.all;

entity bus_fabric_tb is
end bus_fabric_tb;

architecture sim of bus_fabric_tb is
  component bf_ram
    generic (g_mode : integer := 0; g_wait : integer := 0);
    port (
      clk, rst : in  std_logic;
      s_i      : in  bus_m2s_t;
      s_o      : out bus_s2m_t
    );
  end component;

  constant N_M : integer := 3;
  constant N_S : integer := 5;
  type seeds_t is array (0 to N_M - 1) of integer;
  constant SEEDS : seeds_t := (0, 16#2545F491#, 16#1B873593#);
  constant OPS   : seeds_t := (0, 3800, 3000);

  signal clk     : std_logic := '0';
  signal rst     : std_logic := '1';
  signal fin     : std_logic := '0';
  signal m_bus   : m2s_array_t(0 to N_M - 1);
  signal m_rsp   : s2m_array_t(0 to N_M - 1);
  signal s_bus   : m2s_array_t(0 to N_S - 1);
  signal s_rsp   : s2m_array_t(0 to N_S - 1);
  signal dones   : std_logic_vector(0 to N_M - 1);
  signal errs    : int_array_t(0 to N_M - 1) := (others => -1);
  signal opsn    : int_array_t(0 to N_M - 1) := (others => 0);
  signal ticks   : integer := 0;
  signal serial  : std_logic := '1';
  signal irq_tm, irq_rx, irq_tx, irq_sys : std_logic;
  signal irq_in  : std_logic_vector(3 downto 0);
begin
  clk <= '0' when fin = '1' else not clk after 5 ns;
  rst <= '0' after 52 ns;
  irq_in <= '0' & irq_tx & irq_rx & irq_tm;

  masters : for i in 0 to N_M - 1 generate
    cpu_gen : if i = 0 generate
      u_cpu : entity work.bf_cpu
        generic map (g_bytes => 90)
        port map (clk => clk, rst => rst, mo => m_bus(i), mi => m_rsp(i), irq => irq_sys,
                  done => dones(i), errors => errs(i), ticks => ticks);
    end generate cpu_gen;
    stress_gen : if i > 0 generate
      u_stress : entity work.bf_master
        generic map (g_id => i, g_seed => SEEDS(i), g_ops => OPS(i))
        port map (clk => clk, rst => rst, mo => m_bus(i), mi => m_rsp(i),
                  done => dones(i), errors => errs(i), ops => opsn(i));
    end generate stress_gen;
  end generate masters;

  u_fabric : entity work.bf_fabric
    generic map (g_masters => N_M)
    port map (clk => clk, rst => rst, m_in => m_bus, m_out => m_rsp, s_out => s_bus, s_in => s_rsp);

  u_ram_a : bf_ram
    generic map (g_mode => 0, g_wait => 0)
    port map (clk => clk, rst => rst, s_i => s_bus(0), s_o => s_rsp(0));

  u_ram_b : bf_ram
    generic map (g_mode => 1, g_wait => 3)
    port map (clk => clk, rst => rst, s_i => s_bus(1), s_o => s_rsp(1));

  u_timer : entity work.bf_timer
    port map (clk => clk, rst => rst, s_i => s_bus(2), s_o => s_rsp(2), irq => irq_tm);

  u_uart : entity work.bf_uart
    port map (clk => clk, rst => rst, s_i => s_bus(3), s_o => s_rsp(3), txd => serial,
              rxd => serial, irq_rx => irq_rx, irq_tx => irq_tx);

  u_intc : entity work.bf_intc
    port map (clk => clk, rst => rst, s_i => s_bus(4), s_o => s_rsp(4),
              irq_in => irq_in, irq_out => irq_sys);

  -- Protocol monitor: at most one slave selected, at most one master answered.
  mon : process (s_bus, m_rsp)
    variable nsel, nrsp : integer;
  begin
    nsel := 0;
    nrsp := 0;
    for j in s_bus'range loop
      if s_bus(j).req = '1' then
        nsel := nsel + 1;
      end if;
    end loop;
    for i in m_rsp'range loop
      if m_rsp(i).ack = '1' or m_rsp(i).err = '1' then
        nrsp := nrsp + 1;
      end if;
    end loop;
    assert nsel <= 1 report "two slaves selected" severity failure;
    assert nrsp <= 1 report "two masters answered" severity failure;
  end process;

  chk : process
  begin
    wait until rst = '0';
    wait until dones = "111" for 5 ms;
    assert dones = "111" report "masters did not finish" severity failure;
    for i in errs'range loop
      assert errs(i) = 0
        report "master " & integer'image(i) & " reported " & integer'image(errs(i)) & " errors"
        severity failure;
    end loop;
    assert opsn(1) = OPS(1) + 1024 and opsn(2) = OPS(2) + 1024
      report "unexpected operation counts" severity failure;
    assert ticks >= 3 report "timer interrupts missing" severity failure;
    report "master 0 serviced " & integer'image(ticks) & " timer ticks" severity note;
    report "PASS" severity note;
    fin <= '1';
    wait;
  end process;
end sim;
