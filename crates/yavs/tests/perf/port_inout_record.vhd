-- `inout` record ports with `tready` inside on elements of an array of records (`ch(i)`,
-- `ch(i + 1)`) through two levels, one stage per generate lane, each field driven by one side
-- and 'Z' by the other; and eight `inout` ports on one shared resolved bus, every slave driving
-- it each clock. Each formal is an alias of its actual: its drivers drive the actual.
library ieee;
use ieee.std_logic_1164.all;

package port_inout_record_pkg is
  type axs_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector(15 downto 0);
    tready : std_logic;
  end record;
  type axs_arr_t is array (natural range <>) of axs_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.port_inout_record_pkg.all;

entity port_inout_record_stage is
  port (clk : in std_logic; s : inout axs_t; m : inout axs_t);
end;
architecture a of port_inout_record_stage is
  signal v : std_logic := '0';
  signal d : std_logic_vector(15 downto 0) := x"0000";
begin
  s.tready <= not v or m.tready;
  process (clk) begin
    if rising_edge(clk) then
      if v = '0' or m.tready = '1' then
        v <= s.tvalid;
        d <= std_logic_vector(unsigned(s.tdata) + 1);
      end if;
    end if;
  end process;
  m.tvalid <= v;
  m.tdata <= d;
  s.tvalid <= 'Z';
  s.tdata <= (others => 'Z');
  m.tready <= 'Z';
end;

library ieee;
use ieee.std_logic_1164.all;
use work.port_inout_record_pkg.all;

entity port_inout_record_wrap is
  port (clk : in std_logic; s : inout axs_t; m : inout axs_t);
end;
architecture a of port_inout_record_wrap is
begin
  u : entity work.port_inout_record_stage port map (clk, s, m);
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity port_inout_record_slave is
  generic (ID : natural);
  port (clk : in std_logic; sel : in natural; dat : inout std_logic_vector(7 downto 0));
end;
architecture a of port_inout_record_slave is
begin
  process (clk) begin
    if rising_edge(clk) then
      if sel = ID then dat <= std_logic_vector(to_unsigned(ID * 16 + 1, 8));
      else dat <= (others => 'Z'); end if;
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.port_inout_record_pkg.all;

entity port_inout_record is end;
architecture a of port_inout_record is
  constant N : positive := 8;
  signal clk : std_logic := '0';
  signal ch : axs_arr_t(0 to N);
  signal sel : natural := 0;
  signal bus8 : std_logic_vector(7 downto 0);
begin
  g : for i in 0 to N - 1 generate
    u : entity work.port_inout_record_wrap port map (clk, ch(i), ch(i + 1));
    s : entity work.port_inout_record_slave generic map (i) port map (clk, sel, bus8);
  end generate;
  process
    variable x : unsigned(15 downto 0) := x"1234";
    variable sum : unsigned(15 downto 0) := x"0000";
  begin
    ch(0).tready <= 'Z';
    for k in 1 to 40 loop
      x := x(14 downto 0) & (x(15) xor x(13) xor x(12) xor x(10));
      ch(0).tvalid <= x(0);
      ch(0).tdata <= std_logic_vector(x);
      ch(N).tready <= x(2) or x(3);
      sel <= k mod N;
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
      if ch(N).tvalid = '1' and ch(N).tready = '1' then
        sum := (sum rol 3) xor unsigned(ch(N).tdata);
      end if;
      sum := sum + unsigned(bus8);
    end loop;
    report "sum=" & to_hstring(sum);
    wait;
  end process;
end;
