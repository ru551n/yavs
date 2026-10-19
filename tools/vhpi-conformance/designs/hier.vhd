library ieee;
use ieee.std_logic_1164.all;

entity leaf is
  generic (W : natural := 4; TAG : string := "leaf");
  port (clk : in std_logic; d : in std_logic_vector(W - 1 downto 0);
        q : out std_logic_vector(W - 1 downto 0));
end entity;

architecture rtl of leaf is
  signal r : std_logic_vector(W - 1 downto 0) := (others => '0');
  signal wd : integer := W;
begin
  p : process (clk) begin
    if rising_edge(clk) then r <= d; end if;
  end process;
  q <= r;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity hier is end entity;

architecture sim of hier is
  type color is (red, green, blue);
  signal clk  : std_logic := '0';
  signal cnt  : integer := 0;
  signal flag : boolean := false;
  signal bits : bit_vector(3 downto 0) := "1010";
  signal slv  : std_logic_vector(7 downto 0) := x"A5";
  signal rl   : real := 1.5;
  signal ch   : character := 'x';
  signal st   : color := green;
  signal tm   : time := 5 ns;
  signal str  : string(1 to 3) := "abc";
  constant k  : integer := 7;
  signal qa   : std_logic_vector(7 downto 0);
begin
  clock : process begin
    for i in 1 to 4 loop
      wait for 10 ns;
      clk <= not clk;
    end loop;
    wait;
  end process;

  count : process (clk) begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
      flag <= not flag;
      slv <= std_logic_vector(unsigned'(x"01") + unsigned(slv));
    end if;
  end process;

  u_a : entity work.leaf generic map (W => 8) port map (clk => clk, d => slv, q => qa);

  blk : block
    signal inner : std_logic := '1';
  begin
    inner <= clk;
  end block;

  g : for i in 0 to 1 generate
    u : entity work.leaf generic map (W => 4 + i)
      port map (clk => clk, d => slv(4 + i - 1 downto 0), q => open);
  end generate;
end architecture;
