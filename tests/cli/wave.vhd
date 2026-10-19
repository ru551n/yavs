-- Waveform test design for crates/yavs/tests/wave.rs: every kind of recorded object.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity wave_sub is
  port (
    Clk_In : in  std_logic;
    q      : out unsigned(3 downto 0)
  );
end entity;

architecture rtl of wave_sub is
  signal cnt : unsigned(3 downto 0) := (others => '0');
begin
  process (Clk_In) begin
    if rising_edge(Clk_In) then
      cnt <= cnt + 1;
    end if;
  end process;
  q <= cnt;
end architecture;

-- An array of records, in an architecture without processes (the lowering does not take
-- array-of-record signals in process architectures yet).
library ieee;
use ieee.std_logic_1164.all;

entity wave_recs is
end entity;

architecture rtl of wave_recs is
  type pair_t is record
    a : std_logic;
    n : integer;
  end record;
  type pairs_t is array (0 to 1) of pair_t;
  signal recs : pairs_t := (('0', 0), ('1', -1));
begin
end architecture;

entity wave_tb is
end entity;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

architecture sim of wave_tb is
  type state_t is (IDLE, Run_State, DONE);
  type pair_t is record
    a : std_logic;
    n : integer;
  end record;
  type mem_t is array (0 to 3) of std_logic_vector(7 downto 0);
  constant MEM1  : mem_t := (x"00", x"00", x"A5", x"00");
  signal Clk   : std_logic := '0';
  signal count : unsigned(3 downto 0);
  signal state : state_t := IDLE;
  signal flag  : boolean := false;
  signal b     : bit := '0';
  signal bv    : bit_vector(0 to 2) := "001";
  signal i     : integer := 0;
  signal r     : real := 0.0;
  signal t     : time := 0 ns;
  signal rec   : pair_t := ('U', 0);
  signal mem   : mem_t := (others => (others => '0'));
  signal big   : std_logic_vector(99 downto 0) := (others => 'Z');
  type grid_t is array (0 to 1, 0 to 2) of std_logic;
  signal grid  : grid_t := (others => (others => '0'));
  subtype small_t is natural range 0 to 255;
  signal sm    : small_t := 200;
  signal neg   : integer range -8 to 7 := -3;
  signal nat   : natural := 5;
  constant SEL : integer := 1;
begin
  cg : case SEL generate
    when alt1 : 1 =>
      signal cgs : std_logic := '1';
    begin
    end;
    when others =>
    begin
    end;
  end generate;

  ig : if alt_a : SEL = 2 generate
    signal igs : std_logic := '0';
  begin
  elsif alt_b : SEL = 1 generate
    signal igs : std_logic := '1';
  begin
  end generate;

  u1 : entity work.wave_sub port map (Clk_In => Clk, q => count);
  u2 : entity work.wave_recs;

  g : for k in 0 to 1 generate
    signal gs : std_logic := '0';
  begin
    gs <= Clk when k = 1 else '0';
    u : entity work.wave_sub port map (Clk_In => gs, q => open);
  end generate;

  main : process
    variable cnt : natural range 0 to 15 := 0;
    variable vs  : state_t := IDLE;
    variable vr  : real := 0.0;
    variable vl  : std_logic := '0';
    variable vi  : integer range -100 to 100 := 0;
  begin
    for n in 1 to 4 loop
      cnt := cnt + 1;
      Clk <= '1';
      wait for 5 ns;
      Clk <= '0';
      wait for 5 ns;
    end loop;
    state <= Run_State;
    flag <= true;
    b <= '1';
    bv <= "110";
    i <= -7;
    r <= 2.5;
    t <= 3 ns;
    vs := Run_State;
    vr := 2.5;
    vl := 'X';
    vi := -100;
    rec <= ('X', 42);
    mem <= MEM1;
    grid(1, 2) <= '1';
    wait for 1 ns;
    -- several deltas at one time: the file holds the final value
    state <= IDLE;
    wait for 0 ns;
    state <= DONE;
    vs := DONE;
    wait for 1 ns;
    std.env.finish;
  end process;
end architecture;
