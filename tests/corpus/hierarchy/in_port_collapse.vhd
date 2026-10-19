-- difftest: top=in_port_collapse std=2008
-- `in` ports whose actual is a whole signal (an alias of the actual when ports are collapsed)
-- read the actual's value, events, 'last_value, 'last_event, 'stable and edges: a vector of
-- the other direction, a record, a chain through a component, a clocked process on a port.
library ieee;
use ieee.std_logic_1164.all;

package in_port_collapse_pkg is
  type rec_t is record
    n : integer;
    b : std_logic_vector(1 downto 0);
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.in_port_collapse_pkg.all;

entity in_port_collapse_leaf is
  port (clk : in std_logic; v : in std_logic_vector(3 downto 0); r : in rec_t;
        q : out integer);
end;
architecture t of in_port_collapse_leaf is
  signal count : integer := 0;
begin
  process (clk) begin
    if rising_edge(clk) then
      count <= count + r.n;
    end if;
  end process;
  q <= count;
  process (v) begin
    report "v=" & to_string(v) & " v(3)=" & std_logic'image(v(3)) & " event=" &
      boolean'image(v'event) & " last=" & to_string(v'last_value);
  end process;
  process begin
    wait on r;
    report "r.n=" & integer'image(r.n) & " r.b=" & to_string(r.b) & " stable=" &
      boolean'image(r'stable) & " clk'last_event=" & time'image(clk'last_event);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.in_port_collapse_pkg.all;

entity in_port_collapse is end;
architecture t of in_port_collapse is
  component in_port_collapse_leaf is
    port (clk : in std_logic; v : in std_logic_vector(3 downto 0); r : in rec_t;
          q : out integer);
  end component;
  signal clk : std_logic := '0';
  signal v : std_logic_vector(0 to 3) := "0001";
  signal r : rec_t := (0, "00");
  signal q : integer;
begin
  u : in_port_collapse_leaf port map (clk => clk, v => v, r => r, q => q);
  process begin
    wait for 1 ns;
    v <= "1000";
    r <= (3, "10");
    wait for 1 ns;
    for i in 1 to 3 loop
      clk <= '1'; wait for 1 ns;
      clk <= '0'; wait for 1 ns;
    end loop;
    r.n <= 5;
    wait for 1 ns;
    clk <= '1'; wait for 1 ns;
    report "q=" & integer'image(q) & " v'stable=" & boolean'image(v'stable(2 ns));
    wait;
  end process;
end;
