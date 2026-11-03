-- pgo: top=line_window_tb std=2008 stop=1400us
-- Convolution front end: a pixel stream is written into four row banks (RAM arrays with registered
-- read), a 3x3 window is built every pixel and fed to a signed multiply-accumulate pipeline at one
-- pixel per clock. A credit handshake lets the producer reuse banks the reader has released.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package lw_pkg is
  constant PW : positive := 8;
  subtype pix_t is std_logic_vector(PW - 1 downto 0);
  subtype bank_id_t is unsigned(1 downto 0);
  type pix_arr_t is array (natural range <>) of pix_t;
  type row3_t is array (0 to 2) of pix_t;
  type win_t is array (0 to 2) of row3_t;
  type sel3_t is array (0 to 2) of bank_id_t;
  type kern_t is array (0 to 2, 0 to 2) of integer range -8 to 8;
  type kern_arr_t is array (0 to 2) of kern_t;

  -- Edge detect, smooth, sharpen.
  constant KERNELS : kern_arr_t := (
    ((-1, 0, 1), (-2, 0, 2), (-1, 0, 1)),
    ((1, 2, 1), (2, 4, 2), (1, 2, 1)),
    ((0, -1, 0), (-1, 5, -1), (0, -1, 0)));

  function clog2(n : positive) return natural;
  function pix(f : natural; r : natural; c : natural) return natural;
end package;

package body lw_pkg is

  function clog2(n : positive) return natural is
    variable r : natural := 0;
    variable v : positive := 1;
  begin
    while v < n loop
      v := v * 2;
      r := r + 1;
    end loop;
    return r;
  end function;

  -- Test image: frame f, row r, column c.
  function pix(f : natural; r : natural; c : natural) return natural is
  begin
    return (f * 97 + r * 53 + c * 29 + (r * c) mod 61 + ((r / 3) * (c / 5)) mod 31) mod 256;
  end function;

end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.lw_pkg.all;

-- One row of pixels in RAM: synchronous write, registered read.
entity line_bank is
  generic (AW : positive := 6; DW : positive := 8);
  port (
    clk   : in  std_logic;
    we    : in  std_logic;
    waddr : in  unsigned(AW - 1 downto 0);
    wdata : in  std_logic_vector(DW - 1 downto 0);
    raddr : in  unsigned(AW - 1 downto 0);
    rdata : out std_logic_vector(DW - 1 downto 0));
end entity;

architecture rtl of line_bank is
begin
  process (clk)
    type ram_t is array (0 to 2 ** AW - 1) of std_logic_vector(DW - 1 downto 0);
    variable ram : ram_t := (others => (others => '0'));
  begin
    if rising_edge(clk) then
      if we = '1' then
        ram(to_integer(waddr)) := wdata;
      end if;
      rdata <= ram(to_integer(raddr));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.lw_pkg.all;

-- Writes frames row by row into the bank ring, one pixel per clock while it holds a credit.
entity pix_producer is
  generic (W : positive; H : positive; FRAMES : positive; AW : positive);
  port (
    clk, rst : in  std_logic;
    free_n   : in  natural range 0 to 3;
    row_done : out std_logic := '0';
    we       : out std_logic := '0';
    waddr    : out unsigned(AW - 1 downto 0) := (others => '0');
    wbank    : out bank_id_t := "00";
    wdata    : out pix_t := (others => '0');
    fin      : out std_logic := '0');
end entity;

architecture rtl of pix_producer is
begin
  process (clk)
    variable cred             : natural range 0 to 7 := 4;
    variable row, frame, col  : natural := 0;
    variable gb               : bank_id_t := "00";
    variable writing          : boolean := false;
  begin
    if rising_edge(clk) then
      we       <= '0';
      row_done <= '0';
      if rst = '1' then
        cred := 4; row := 0; frame := 0; col := 0; gb := "00"; writing := false;
        fin <= '0';
      else
        cred := cred + free_n;
        if not writing and frame < FRAMES and cred > 0 then
          writing := true;
          cred    := cred - 1;
          col     := 0;
        end if;
        if writing then
          we    <= '1';
          waddr <= to_unsigned(col, AW);
          wbank <= gb;
          wdata <= std_logic_vector(to_unsigned(pix(frame, row, col), PW));
          if col = W - 1 then
            row_done <= '1';
            writing  := false;
            gb       := gb + 1;
            if row = H - 1 then
              row   := 0;
              frame := frame + 1;
            else
              row := row + 1;
            end if;
            if frame < FRAMES and cred > 0 then
              writing := true;
              cred    := cred - 1;
              col     := 0;
            end if;
          else
            col := col + 1;
          end if;
        end if;
        if frame = FRAMES and not writing then
          fin <= '1';
        end if;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.lw_pkg.all;

-- Read side control: waits for finished rows, reads the three newest rows column by column,
-- and hands bank credits back to the producer.
entity win_reader is
  generic (W : positive; H : positive; FRAMES : positive; AW : positive);
  port (
    clk, rst : in  std_logic;
    row_done : in  std_logic;
    free_n   : out natural range 0 to 3 := 0;
    raddr    : out unsigned(AW - 1 downto 0) := (others => '0');
    rd_v     : out std_logic := '0';
    sel      : out sel3_t := (others => "00");
    kidx     : out natural range 0 to 2 := 0);
end entity;

architecture rtl of win_reader is
  type st_t is (ST_IDLE, ST_READ);
begin
  process (clk)
    variable st                         : st_t := ST_IDLE;
    variable done_cnt, g, r, frame      : natural := 0;
    variable col                        : natural range 0 to W - 1 := 0;
    variable gb                         : bank_id_t := "00";
  begin
    if rising_edge(clk) then
      free_n <= 0;
      rd_v   <= '0';
      if rst = '1' then
        st := ST_IDLE; done_cnt := 0; g := 0; r := 0; frame := 0; col := 0; gb := "00";
      else
        if row_done = '1' then
          done_cnt := done_cnt + 1;
        end if;
        if st = ST_IDLE and frame < FRAMES and done_cnt > g then
          if r < 2 then
            -- First two rows of a frame only fill the banks.
            r  := r + 1;
            g  := g + 1;
            gb := gb + 1;
          else
            st  := ST_READ;
            col := 0;
          end if;
        end if;
        if st = ST_READ then
          raddr  <= to_unsigned(col, AW);
          rd_v   <= '1';
          sel(0) <= gb - 2;
          sel(1) <= gb - 1;
          sel(2) <= gb;
          kidx   <= frame mod 3;
          if col = W - 1 then
            gb := gb + 1;
            g  := g + 1;
            if r = H - 1 then
              free_n <= 3;
              r      := 0;
              frame  := frame + 1;
              st     := ST_IDLE;
            else
              free_n <= 1;
              r      := r + 1;
              if done_cnt > g then
                col := 0;
              else
                st := ST_IDLE;
              end if;
            end if;
          else
            col := col + 1;
          end if;
        end if;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.lw_pkg.all;

-- Banks, reader and the window / multiply-accumulate pipeline.
entity line_window is
  generic (W : positive := 60; H : positive := 26; FRAMES : positive := 4; AW : positive := 6);
  port (
    clk, rst : in  std_logic;
    row_done : in  std_logic;
    free_n   : out natural range 0 to 3;
    we       : in  std_logic;
    waddr    : in  unsigned(AW - 1 downto 0);
    wbank    : in  bank_id_t;
    wdata    : in  pix_t;
    out_v    : out std_logic := '0';
    out_px   : out pix_t := (others => '0'));
end entity;

architecture rtl of line_window is
  signal bank_we  : std_logic_vector(0 to 3);
  signal bank_q   : pix_arr_t(0 to 3) := (others => (others => '0'));
  signal raddr    : unsigned(AW - 1 downto 0) := (others => '0');
  signal rd_v     : std_logic;
  signal sel      : sel3_t := (others => "00");
  signal kidx     : natural range 0 to 2;
  -- stage 1: control delayed to line up with the registered RAM read
  signal v1       : std_logic := '0';
  signal c1       : unsigned(AW - 1 downto 0) := (others => '0');
  signal sel1     : sel3_t := (others => "00");
  signal k1       : natural range 0 to 2;
  -- stage 2: window
  signal win      : win_t := (others => (others => (others => '0')));
  signal win_bus  : std_logic_vector(9 * PW - 1 downto 0);
  signal wv2      : std_logic := '0';
  signal k2       : natural range 0 to 2;
  -- stages 3-6
  type prod_arr_t is array (0 to 8) of signed(16 downto 0);
  type rsum_arr_t is array (0 to 2) of signed(19 downto 0);
  signal prod     : prod_arr_t := (others => (others => '0'));
  signal rsum     : rsum_arr_t := (others => (others => '0'));
  signal total    : signed(19 downto 0) := (others => '0');
  signal vp       : std_logic_vector(3 downto 0) := (others => '0');
begin
  rd : entity work.win_reader
    generic map (W => W, H => H, FRAMES => FRAMES, AW => AW)
    port map (clk => clk, rst => rst, row_done => row_done, free_n => free_n, raddr => raddr,
              rd_v => rd_v, sel => sel, kidx => kidx);

  banks : for b in 0 to 3 generate
    bank_we(b) <= we when wbank = to_unsigned(b, 2) else '0';
    u : entity work.line_bank
      generic map (AW => AW, DW => PW)
      port map (clk => clk, we => bank_we(b), waddr => waddr, wdata => wdata, raddr => raddr,
                rdata => bank_q(b));
  end generate;

  stage1 : process (clk)
  begin
    if rising_edge(clk) then
      v1   <= rd_v;
      c1   <= raddr;
      sel1 <= sel;
      k1   <= kidx;
    end if;
  end process;

  stage2 : process (clk)
  begin
    if rising_edge(clk) then
      if v1 = '1' then
        for i in 0 to 2 loop
          win(i)(0) <= win(i)(1);
          win(i)(1) <= win(i)(2);
          win(i)(2) <= bank_q(to_integer(sel1(i)));
        end loop;
      end if;
      if v1 = '1' and to_integer(c1) >= 2 then
        wv2 <= '1';
      else
        wv2 <= '0';
      end if;
      k2 <= k1;
    end if;
  end process;

  bus_rows : for i in 0 to 2 generate
    bus_cols : for j in 0 to 2 generate
      win_bus(8 * (3 * i + j) + 7 downto 8 * (3 * i + j)) <= win(i)(j);
    end generate;
  end generate;

  mult : process (clk)
    variable px : signed(PW downto 0);
    variable kk : signed(7 downto 0);
  begin
    if rising_edge(clk) then
      for k in 0 to 8 loop
        px      := signed('0' & win_bus(8 * k + 7 downto 8 * k));
        kk      := to_signed(KERNELS(k2)(k / 3, k mod 3), 8);
        prod(k) <= px * kk;
      end loop;
    end if;
  end process;

  add_rows : process (clk)
  begin
    if rising_edge(clk) then
      for i in 0 to 2 loop
        rsum(i) <= resize(prod(3 * i), 20) + resize(prod(3 * i + 1), 20) + resize(prod(3 * i + 2), 20);
      end loop;
    end if;
  end process;

  add_all : process (clk)
  begin
    if rising_edge(clk) then
      total <= rsum(0) + rsum(1) + rsum(2);
    end if;
  end process;

  scale : process (clk)
    variable sc : signed(19 downto 0);
  begin
    if rising_edge(clk) then
      sc := shift_right(total, 4) + 128;
      if sc < 0 then
        out_px <= (others => '0');
      elsif sc > 255 then
        out_px <= (others => '1');
      else
        out_px <= std_logic_vector(sc(PW - 1 downto 0));
      end if;
    end if;
  end process;

  valid_pipe : process (clk)
  begin
    if rising_edge(clk) then
      vp <= vp(2 downto 0) & wv2;
    end if;
  end process;
  out_v <= vp(3);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.lw_pkg.all;

entity line_window_tb is
end entity;

architecture sim of line_window_tb is
  constant W      : positive := 60;
  constant H      : positive := 26;
  constant FRAMES : positive := 80;
  constant AW     : positive := clog2(W);
  constant PERIOD : time := 10 ns;

  signal clk, rst : std_logic := '0';
  signal fin      : boolean := false;
  signal free_n   : natural range 0 to 3;
  signal row_done, we, pfin : std_logic;
  signal waddr    : unsigned(AW - 1 downto 0) := (others => '0');
  signal wbank    : bank_id_t := "00";
  signal wdata    : pix_t := (others => '0');
  signal out_v    : std_logic;
  signal out_px   : pix_t;
  signal n_out    : natural := 0;

  -- Reference: direct integer convolution on the test image.
  function model(f, r, c : natural) return natural is
    variable acc : integer := 0;
    variable sc  : integer;
  begin
    for i in 0 to 2 loop
      for j in 0 to 2 loop
        acc := acc + KERNELS(f mod 3)(i, j) * pix(f, r - 2 + i, c - 2 + j);
      end loop;
    end loop;
    sc := (acc - (acc mod 16)) / 16 + 128;
    if sc < 0 then
      return 0;
    elsif sc > 255 then
      return 255;
    end if;
    return sc;
  end function;
begin
  clk <= not clk after PERIOD / 2 when not fin;
  rst <= '1', '0' after 4 * PERIOD;

  prod : entity work.pix_producer
    generic map (W => W, H => H, FRAMES => FRAMES, AW => AW)
    port map (clk => clk, rst => rst, free_n => free_n, row_done => row_done, we => we,
              waddr => waddr, wbank => wbank, wdata => wdata, fin => pfin);

  dut : entity work.line_window
    generic map (W => W, H => H, FRAMES => FRAMES, AW => AW)
    port map (clk => clk, rst => rst, row_done => row_done, free_n => free_n, we => we,
              waddr => waddr, wbank => wbank, wdata => wdata, out_v => out_v, out_px => out_px);

  check : process (clk)
    variable f, r, c : natural := 0;
    variable n       : natural := 0;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        f := 0; r := 2; c := 2; n := 0;
      elsif out_v = '1' then
        assert f < FRAMES report "output after the last frame" severity failure;
        assert to_integer(unsigned(out_px)) = model(f, r, c)
          report "pixel f=" & integer'image(f) & " r=" & integer'image(r) & " c=" & integer'image(c)
                 & " got " & integer'image(to_integer(unsigned(out_px)))
                 & " expected " & integer'image(model(f, r, c)) severity failure;
        n := n + 1;
        if c = W - 1 then
          c := 2;
          if r = H - 1 then
            r := 2;
            f := f + 1;
          else
            r := r + 1;
          end if;
        else
          c := c + 1;
        end if;
      end if;
      n_out <= n;
    end if;
  end process;

  main : process
  begin
    wait until n_out = FRAMES * (H - 2) * (W - 2) for 1350 us;
    wait for 20 * PERIOD;
    assert n_out = FRAMES * (H - 2) * (W - 2)
      report "windows checked: " & integer'image(n_out) severity failure;
    fin <= true;
    report "PASS" severity note;
    wait;
  end process;
end architecture;
