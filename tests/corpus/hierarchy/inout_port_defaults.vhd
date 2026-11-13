-- difftest: top=inout_port_defaults std=2008 ignore=order
-- `inout` ports whose defaults are not their actuals' initial values, and fields a port never
-- drives: an unresolved integer port with default 7 that is its actual's only source and is
-- driven from 2 ns (the actual is 7 from initialisation), a std_logic port with default 'H',
-- AXI-stream style records whose ports default to all 'Z' and drive only the fields they own
-- (no 'Z' drivers), an actual array initialised to '0' whose ports default to 'U' (the
-- drivers start at 'U', so the channel is 'U' until they drive), and a 'Z' default against a
-- '-' (resolves to 'X').
-- ignore=order: the readers of the levels report in the same delta.
library ieee;
use ieee.std_logic_1164.all;

package iopd_pkg is
  type axs_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector(7 downto 0);
    tready : std_logic;
  end record;
  constant AXS_Z : axs_t := (tvalid => 'Z', tdata => (others => 'Z'), tready => 'Z');
  type axs_arr_t is array (natural range <>) of axs_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.iopd_pkg.all;

entity iopd_stage is
  port (clk : in std_logic; s : inout axs_t := AXS_Z; m : inout axs_t := AXS_Z);
end;
architecture t of iopd_stage is
  signal v : std_logic := '0';
  signal d : std_logic_vector(7 downto 0) := x"00";
begin
  s.tready <= not v or m.tready;
  process (clk) begin
    if rising_edge(clk) then
      if v = '0' or m.tready = '1' then
        v <= s.tvalid;
        d <= std_logic_vector(unsigned(s.tdata) + 3);
      end if;
    end if;
  end process;
  m.tvalid <= v;
  m.tdata <= d;
end;

library ieee;
use ieee.std_logic_1164.all;

entity iopd_scalars is
  port (n : inout integer := 7; h : inout std_logic := 'H'; z : inout std_logic := 'Z');
end;
architecture t of iopd_scalars is
begin
  drv : process
  begin
    wait for 2 ns;
    n <= 11;
    h <= '0';
    wait;
  end process;
  rd : process (n, h, z)
  begin
    report "scalars n=" & integer'image(n) & " h=" & to_string(h) & " z=" & to_string(z);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity iopd_reg is
  port (clk : in std_logic; s : inout std_logic; m : inout std_logic);
end;
architecture t of iopd_reg is
begin
  process (clk) begin
    if rising_edge(clk) then
      m <= s;
    end if;
  end process;
  s <= 'Z';
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.iopd_pkg.all;

entity inout_port_defaults is end;
architecture t of inout_port_defaults is
  signal clk : std_logic := '0';
  signal n : integer := 0;
  signal h : std_logic;
  signal z : std_logic;
  signal ch : axs_arr_t(0 to 3);
  signal r : std_logic_vector(0 to 3) := (others => '0');
begin
  sc : entity work.iopd_scalars port map (n, h, z);
  drv_z : z <= '-', '1' after 3 ns;
  g : for i in 0 to 2 generate
    u : entity work.iopd_stage port map (clk, ch(i), ch(i + 1));
    rg : entity work.iopd_reg port map (clk, r(i), r(i + 1));
  end generate;

  rd : process (n, h, z, r)
  begin
    report "top n=" & integer'image(n) & " h=" & to_string(h) & " z=" & to_string(z)
      & " r=" & to_string(r);
  end process;

  src : process
    variable x : unsigned(7 downto 0) := x"5a";
    variable sum : unsigned(15 downto 0) := x"0000";
  begin
    ch(0).tvalid <= '0';
    ch(0).tdata <= x"00";
    ch(3).tready <= '1';
    r(0) <= '1';
    wait for 5 ns;
    for k in 1 to 30 loop
      x := x(6 downto 0) & (x(7) xor x(5) xor x(4) xor x(3));
      if ch(0).tvalid /= '1' or ch(0).tready = '1' then
        ch(0).tvalid <= x(0);
        ch(0).tdata <= std_logic_vector(x);
      end if;
      ch(3).tready <= x(1) or x(2);
      r(0) <= x(3);
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      if ch(3).tvalid = '1' and ch(3).tready = '1' then
        sum := (sum rol 3) xor resize(unsigned(ch(3).tdata), 16);
      end if;
    end loop;
    report "sum=" & to_hstring(sum) & " ch(1)=" & to_hstring(ch(1).tdata) & " ready="
      & to_string(ch(1).tready);
    assert n = 11 and h = '0' and z = '1' report "scalars" severity failure;
    wait;
  end process;
end;
