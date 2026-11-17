-- `--cycle-strict` errors (A40): one design per code, each with a testbench top (exempt) and
-- a design below it holding exactly the construct of that code. The snapshots are
-- `tests/cycle/strict/<code>.stderr` (`cycle_strict_gives_each_code_with_what_where_why_and_fix`).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- E4101: a `while` loop, in the subset but not lowered yet.
entity d_e4101 is
  port (clk : in std_ulogic; a : in unsigned(3 downto 0); q : out unsigned(3 downto 0));
end entity;
architecture rtl of d_e4101 is
begin
  p : process (clk)
    variable v : unsigned(3 downto 0);
  begin
    if rising_edge(clk) then
      v := a;
      while v > 3 loop
        v := v - 3;
      end loop;
      q <= v;
    end if;
  end process;
end architecture;

-- E4110: a wait on a signal.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4110 is
  port (a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4110 is
begin
  p : process
  begin
    wait on a;
    q <= a;
  end process;
end architecture;

-- E4111: a delay in the design.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4111 is
  port (a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4111 is
begin
  q <= a after 1 ns;
end architecture;

-- E4112: signal history.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4112 is
  port (a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4112 is
begin
  q <= a'last_value;
end architecture;

-- E4113: a real value at run time.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4113 is
  port (clk : in std_ulogic; a : in integer; q : out integer);
end entity;
architecture rtl of d_e4113 is
begin
  p : process (clk)
    variable acc : real := 0.0;
  begin
    if rising_edge(clk) then
      acc := acc + real(a);
      q <= integer(acc);
    end if;
  end process;
end architecture;

-- E4114: a method of a protected object.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4114 is
  port (clk, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4114 is
  type flag_t is protected
    impure function get return std_ulogic;
  end protected;
  type flag_t is protected body
    impure function get return std_ulogic is
    begin
      return '1';
    end function;
  end protected body;
  shared variable flag : flag_t;
begin
  p : process (clk)
  begin
    if rising_edge(clk) then
      q <= a and flag.get;
    end if;
  end process;
end architecture;

-- E4115: a postponed process.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4115 is
  port (a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4115 is
begin
  p : postponed process (a)
  begin
    q <= a;
  end process;
end architecture;

-- E4117: a call of a function without a body.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4117 is
  port (a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4117 is
  function outside(x : std_ulogic) return std_ulogic;
  attribute foreign of outside : function is "VHPIDIRECT outside";
begin
  q <= outside(a);
end architecture;

-- E4118: a 'Z'.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4118 is
  port (en, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4118 is
begin
  q <= a when en = '1' else 'Z';
end architecture;

-- E4119: a latch.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4119 is
  port (en, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4119 is
begin
  p : process (en, a)
  begin
    if en = '1' then
      q <= a;
    end if;
  end process;
end architecture;

-- E4120: a variable read before it is assigned.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4120 is
  port (en, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4120 is
begin
  p : process (en, a)
    variable v : std_ulogic;
  begin
    q <= v;
    if en = '1' then
      v := a;
    end if;
  end process;
end architecture;

-- E4121: an incomplete sensitivity list.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4121 is
  port (a, b : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4121 is
begin
  p : process (a)
  begin
    q <= a and b;
  end process;
end architecture;

-- E4122: a combinational loop.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4122 is
  port (a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4122 is
  signal x, y : std_ulogic;
begin
  x <= a xor y;
  y <= not x;
  q <= y;
end architecture;

-- E4123: an edge test with an `else`.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4123 is
  port (clk, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4123 is
begin
  p : process (clk)
  begin
    if rising_edge(clk) then
      q <= a;
    else
      q <= '0';
    end if;
  end process;
end architecture;

-- E4124: two drivers.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4124 is
  port (clk, a, b : in std_ulogic; q : out std_logic);
end entity;
architecture rtl of d_e4124 is
begin
  one : process (clk)
  begin
    if rising_edge(clk) then
      q <= a;
    end if;
  end process;
  two : process (clk)
  begin
    if rising_edge(clk) then
      q <= b;
    end if;
  end process;
end architecture;

-- E4125: a gated clock.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4125 is
  port (clk, en, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4125 is
  signal gclk : std_ulogic;
begin
  gclk <= clk and en;
  p : process (gclk)
  begin
    if rising_edge(gclk) then
      q <= a;
    end if;
  end process;
end architecture;

-- E4127: the testbench reads the history of a register.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4127 is
  port (clk, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4127 is
begin
  p : process (clk)
  begin
    if rising_edge(clk) then
      q <= a;
    end if;
  end process;
end architecture;

-- E4129: logic feeding a latch outside the region.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4129 is
  port (a, b, en : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4129 is
  signal c : std_ulogic;
begin
  c <= a and b;
  latch : process (en, c)
  begin
    if en = '1' then
      q <= c;
    end if;
  end process;
end architecture;

-- E4130: logic between a clock and a gated copy made by the testbench.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4130 is
  port (clk, gclk, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4130 is
  signal r, c : std_ulogic := '0';
begin
  p1 : process (clk)
  begin
    if rising_edge(clk) then
      r <= a;
    end if;
  end process;
  c <= not r;
  p2 : process (gclk)
  begin
    if rising_edge(gclk) then
      q <= c;
    end if;
  end process;
end architecture;

-- E4133: a memory written in an asynchronous reset branch.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4133 is
  port (clk, rst : in std_ulogic; a : in unsigned(12 downto 0); d : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4133 is
  type mem_t is array (0 to 8191) of std_ulogic;
  signal mem : mem_t := (others => '0');
begin
  p : process (clk, rst)
  begin
    if rst = '1' then
      mem(0) <= '0';
    elsif rising_edge(clk) then
      mem(to_integer(a)) <= d;
      q <= mem(to_integer(a));
    end if;
  end process;
end architecture;

-- E4134: a vector driven partly by a behavioural model of the design.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4134 is
  port (a : in std_ulogic; q : out std_ulogic_vector(1 downto 0));
end entity;
architecture rtl of d_e4134 is
  signal v : std_ulogic_vector(1 downto 0);
begin
  v(0) <= not a;
  model : process
  begin
    v(1) <= '0';
    wait for 5 ns;
    v(1) <= '1';
    wait;
  end process;
  q <= v;
end architecture;

-- E4135: a vector driven partly by the testbench through the port.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4135 is
  port (a : in std_ulogic; q : out std_logic_vector(1 downto 0));
end entity;
architecture rtl of d_e4135 is
begin
  q(0) <= not a;
end architecture;

-- E4137: a metavalue literal the testbench reads (with --cycle-exact-meta).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4137 is
  port (en, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4137 is
begin
  q <= a when en = '1' else 'X';
end architecture;

-- E4138: an external name.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4138 is
  port (q : out std_ulogic);
end entity;
architecture rtl of d_e4138 is
begin
  q <= <<signal .t_e4138.marker : std_ulogic>>;
end architecture;

-- E4139: an `exit` of an outer loop.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4139 is
  port (clk : in std_ulogic; a : in std_ulogic_vector(3 downto 0); q : out natural);
end entity;
architecture rtl of d_e4139 is
begin
  p : process (clk)
    variable n : natural;
  begin
    if rising_edge(clk) then
      n := 0;
      outer : for i in 0 to 3 loop
        for j in 0 to 3 loop
          exit outer when a(j) = '1';
          n := n + 1;
        end loop;
      end loop;
      q <= n;
    end if;
  end process;
end architecture;

-- The testbenches: a clock, a stimulus process (behavioural) and the design.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4101 is
end entity;
architecture tb of t_e4101 is
  signal clk : std_ulogic := '0';
  signal a, q : unsigned(3 downto 0) := "0000";
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4101 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= "1010";
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4110 is
end entity;
architecture tb of t_e4110 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4110 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4111 is
end entity;
architecture tb of t_e4111 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4111 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4112 is
end entity;
architecture tb of t_e4112 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4112 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4113 is
end entity;
architecture tb of t_e4113 is
  signal clk : std_ulogic := '0';
  signal a, q : integer := 0;
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4113 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= 3;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4114 is
end entity;
architecture tb of t_e4114 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4114 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4115 is
end entity;
architecture tb of t_e4115 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4115 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;


library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4117 is
end entity;
architecture tb of t_e4117 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4117 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4118 is
end entity;
architecture tb of t_e4118 is
  signal clk : std_ulogic := '0';
  signal en, a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4118 port map (en, a, q);
  stim : process
  begin
    wait for 10 ns;
    en <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4119 is
end entity;
architecture tb of t_e4119 is
  signal clk : std_ulogic := '0';
  signal en, a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4119 port map (en, a, q);
  stim : process
  begin
    wait for 10 ns;
    en <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4120 is
end entity;
architecture tb of t_e4120 is
  signal clk : std_ulogic := '0';
  signal en, a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4120 port map (en, a, q);
  stim : process
  begin
    wait for 10 ns;
    en <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4121 is
end entity;
architecture tb of t_e4121 is
  signal clk : std_ulogic := '0';
  signal a, b, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4121 port map (a, b, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4122 is
end entity;
architecture tb of t_e4122 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4122 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4123 is
end entity;
architecture tb of t_e4123 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4123 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4124 is
end entity;
architecture tb of t_e4124 is
  signal clk : std_ulogic := '0';
  signal a, b : std_ulogic := '0'; signal q : std_logic;
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4124 port map (clk, a, b, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4125 is
end entity;
architecture tb of t_e4125 is
  signal clk : std_ulogic := '0';
  signal en, a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4125 port map (clk, en, a, q);
  stim : process
  begin
    wait for 10 ns;
    en <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4127 is
end entity;
architecture tb of t_e4127 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4127 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1'; wait for 20 ns; report "q changed " & time'image(q'last_event) & " ago";
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4129 is
end entity;
architecture tb of t_e4129 is
  signal clk : std_ulogic := '0';
  signal a, b, en, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4129 port map (a, b, en, q);
  stim : process
  begin
    wait for 10 ns;
    en <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4130 is
end entity;
architecture tb of t_e4130 is
  signal clk : std_ulogic := '0';
  signal en, gclk, a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  gclk <= clk and en;
  dut : entity work.d_e4130 port map (clk, gclk, a, q);
  stim : process
  begin
    wait for 10 ns;
    en <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4133 is
end entity;
architecture tb of t_e4133 is
  signal clk : std_ulogic := '0';
  signal rst, d, q : std_ulogic := '0'; signal a : unsigned(12 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4133 port map (clk, rst, a, d, q);
  stim : process
  begin
    wait for 10 ns;
    d <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4135 is
end entity;
architecture tb of t_e4135 is
  signal clk : std_ulogic := '0';
  signal a : std_ulogic := '0'; signal q : std_logic_vector(1 downto 0);
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4135 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    q(1) <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4137 is
end entity;
architecture tb of t_e4137 is
  signal clk : std_ulogic := '0';
  signal en, a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4137 port map (en, a, q);
  stim : process
  begin
    wait for 10 ns;
    en <= '1'; wait for 1 ns; report "q = " & std_ulogic'image(q);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4138 is
end entity;
architecture tb of t_e4138 is
  signal clk : std_ulogic := '0';
  signal marker, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4138 port map (q);
  stim : process
  begin
    wait for 10 ns;
    marker <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4139 is
end entity;
architecture tb of t_e4139 is
  signal clk : std_ulogic := '0';
  signal a : std_ulogic_vector(3 downto 0) := "0000"; signal q : natural;
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4139 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= "0100";
    wait;
  end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4134 is
end entity;
architecture tb of t_e4134 is
  signal a : std_ulogic := '0';
  signal q : std_ulogic_vector(1 downto 0);
begin
  dut : entity work.d_e4134 port map (a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

-- E4126: an `out` parameter associated part by part.
library ieee;
use ieee.std_logic_1164.all;
entity d_e4126 is
  port (a : in std_ulogic_vector(1 downto 0); q, r : out std_ulogic);
end entity;
architecture rtl of d_e4126 is
  procedure two(x : in std_ulogic_vector; y : out std_ulogic_vector) is
  begin
    y := x;
  end procedure;
begin
  p : process (a)
    variable u, w : std_ulogic;
  begin
    two(a, y(1) => u, y(0) => w);
    q <= u;
    r <= w;
  end process;
end architecture;

-- E4131: logic computing a clock from a register of the region, for a process outside.
library ieee;
use ieee.std_logic_1164.all;
entity d_e4131 is
  port (clk, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4131 is
  signal r, gclk : std_ulogic := '0';
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= a;
    end if;
  end process;
  gclk <= clk and r;
  user : process (gclk)
  begin
    if rising_edge(gclk) then
      q <= 'Z';
    end if;
  end process;
end architecture;

-- E4132: a register reset by another register, used as the reset of a third.
library ieee;
use ieee.std_logic_1164.all;
entity d_e4132 is
  port (clk, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture rtl of d_e4132 is
  signal rr, r2 : std_ulogic := '0';
begin
  r1 : process (clk)
  begin
    if rising_edge(clk) then
      rr <= a;
    end if;
  end process;
  reg2 : process (clk, rr)
  begin
    if rr = '1' then
      r2 <= '0';
    elsif rising_edge(clk) then
      r2 <= not r2;
    end if;
  end process;
  reg3 : process (clk, r2)
  begin
    if r2 = '1' then
      q <= '0';
    elsif rising_edge(clk) then
      q <= a;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4126 is
end entity;
architecture tb of t_e4126 is
  signal clk : std_ulogic := '0';
  signal a : std_ulogic_vector(1 downto 0) := "00"; signal q, r : std_ulogic;
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4126 port map (a, q, r);
  stim : process
  begin
    wait for 10 ns;
    a <= "10";
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4131 is
end entity;
architecture tb of t_e4131 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4131 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4132 is
end entity;
architecture tb of t_e4132 is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4132 port map (clk, a, q);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;
