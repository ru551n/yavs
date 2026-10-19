-- difftest: top=in_port_collapse_part std=2008 ref=hand ignore=order
-- `in` ports whose actual is part of a signal (an element, a slice, an element of a vector
-- of the other direction), through a component, read the actual's value, events,
-- 'last_value, 'stable, 'delayed and edges: a chain of cells over one vector as in a
-- generate, clocked from an element of a clock vector. ref=hand: `n'delayed` starts at the
-- port's initial value, its actual's 0 (LRM 14.7.5.2; three of the four references), where
-- the reference simulator starts it at INTEGER'LEFT; the same-cycle report order differs
-- between the references.
library ieee;
use ieee.std_logic_1164.all;

entity in_port_collapse_part_cell is
  port (clk : in std_logic; d : in std_logic; s : in std_logic_vector(1 downto 0);
        n : in integer; q : out std_logic);
end;
architecture t of in_port_collapse_part_cell is
begin
  process (clk) begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
  process (s, n) begin
    report "s=" & to_string(s) & " last=" & to_string(s'last_value) & " event=" &
      boolean'image(s'event) & " stable=" & boolean'image(s'stable) & " n=" &
      integer'image(n) & " n'delayed=" & integer'image(n'delayed(1 ns));
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity in_port_collapse_part is end;
architecture t of in_port_collapse_part is
  component in_port_collapse_part_cell is
    port (clk : in std_logic; d : in std_logic; s : in std_logic_vector(1 downto 0);
          n : in integer; q : out std_logic);
  end component;
  signal clks : std_logic_vector(0 to 1) := "00";
  signal link : std_logic_vector(0 to 3) := "1000";
  signal bus4 : std_logic_vector(7 downto 0) := x"00";
  type int_arr is array (0 to 2) of integer;
  signal nums : int_arr := (others => 0);
begin
  g : for i in 0 to 2 generate
    u : in_port_collapse_part_cell port map (
      clk => clks(1), d => link(i), s => bus4(2 * i + 1 downto 2 * i), n => nums(i),
      q => link(i + 1));
  end generate;
  process begin
    wait for 1 ns;
    bus4 <= x"1b";
    nums(1) <= 7;
    for k in 1 to 4 loop
      clks(1) <= '1'; wait for 1 ns;
      report "link=" & to_string(link);
      clks(1) <= '0'; wait for 1 ns;
      clks(0) <= not clks(0);
    end loop;
    bus4(3 downto 2) <= "01";
    nums(2) <= 5;
    wait for 2 ns;
    wait;
  end process;
end;
