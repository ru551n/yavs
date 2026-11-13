-- Record ports with a null field (`tuser` of width 0) on elements of an array of records
-- (`ch(i)`, `ch(i + 1)`) through two levels of hierarchy, one stage per generate lane: each
-- formal is an alias of its element, the null field included.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package port_null_record_pkg is
  type axis_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector(15 downto 0);
    tlast  : std_logic;
    tuser  : std_logic_vector;
  end record;
  type axis_arr_t is array (natural range <>) of axis_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.port_null_record_pkg.all;

entity port_null_record_stage is
  port (clk : in std_logic; s : in axis_t; m : out axis_t);
end;
architecture a of port_null_record_stage is
  signal r : axis_t(tuser(1 to 0)) := ('0', x"0000", '0', "");
begin
  process (clk) begin
    if rising_edge(clk) then
      r <= (s.tvalid, std_logic_vector(unsigned(s.tdata) + 1), s.tlast, "");
    end if;
  end process;
  m <= r;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.port_null_record_pkg.all;

entity port_null_record_wrap is
  port (clk : in std_logic; s : in axis_t; m : out axis_t);
end;
architecture a of port_null_record_wrap is
begin
  u : entity work.port_null_record_stage port map (clk, s, m);
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.port_null_record_pkg.all;

entity port_null_record is end;
architecture a of port_null_record is
  constant N : positive := 8;
  signal clk : std_logic := '0';
  signal ch : axis_arr_t(0 to N)(tuser(1 to 0));
begin
  g : for i in 0 to N - 1 generate
    u : entity work.port_null_record_wrap port map (clk, ch(i), ch(i + 1));
  end generate;
  process
    variable x : unsigned(15 downto 0) := x"1234";
    variable sum : unsigned(15 downto 0) := x"0000";
  begin
    for k in 1 to 40 loop
      x := x(14 downto 0) & (x(15) xor x(13) xor x(12) xor x(10));
      ch(0) <= (x(0), std_logic_vector(x), x(1), "");
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
      if k > N then
        sum := (sum rol 3) xor unsigned(ch(N).tdata);
        if ch(N).tlast = '1' then sum := sum + 1; end if;
      end if;
    end loop;
    report "sum=" & to_hstring(sum);
    wait;
  end process;
end;
