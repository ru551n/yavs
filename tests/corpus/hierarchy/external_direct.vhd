-- difftest: top=external_direct std=2008 stop=100ns
-- External names used directly in sequential and concurrent code (LRM 8.7): reads, waits,
-- sensitivity lists and drivers of signals of an instance elaborated after the process,
-- external constants (of a package too), record signals, and aliases of parts of external names.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package types is
  type pair is record
    a : natural;
    b : std_logic_vector(3 downto 0);
  end record;
  constant limit : natural := 42;
end package;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.types.all;
entity dut is
  generic (width : positive := 8);
  port (clk : in std_logic);
end;
architecture a of dut is
  constant depth : natural := width * 2;
  constant tag : string := "dut-" & "a";
  signal count : natural := 0;
  signal req : bit := '0';
  signal data : std_logic_vector(width - 1 downto 0) := (others => '0');
  signal rec : pair := (a => 1, b => x"5");
begin
  process (clk) begin
    if rising_edge(clk) then
      count <= count + 1;
      rec.a <= rec.a + 2;
      if req = '1' then
        data <= std_logic_vector(unsigned(data) + 3);
      end if;
    end if;
  end process;
  process (req) begin
    report "dut: req=" & bit'image(req);
  end process;
end;
library ieee;
use ieee.std_logic_1164.all;
use work.types.all;
entity external_direct is end;
architecture tb of external_direct is
  signal clk : std_logic := '0';
  signal done : boolean := false;
  signal mirror : natural;
begin
  clk <= not clk after 5 ns when not done;
  -- Before the instance: the reference simulators bind these once the design exists.
  stim : process
    variable w : natural := <<constant .external_direct.u.width : positive>>;
  begin
    wait for 1 ns;
    report "width=" & integer'image(w) & " depth="
      & integer'image(<<constant .external_direct.u.depth : natural>>)
      & " tag=" & <<constant .external_direct.u.tag : string>>
      & " limit=" & integer'image(<<constant @work.types.limit : natural>>);
    wait for 11 ns;
    <<signal .external_direct.u.req : bit>> <= '1';
    wait on <<signal .external_direct.u.data : std_logic_vector(7 downto 0)>>;
    wait for 1 ns;
    report "data=" & to_hstring(<<signal .external_direct.u.data : std_logic_vector(7 downto 0)>>)
      & " bit0=" & std_logic'image(<<signal .external_direct.u.data : std_logic_vector>>(0));
    wait until <<signal .external_direct.u.data : std_logic_vector(7 downto 0)>>(3 downto 0) = x"6";
    wait for 1 ns;
    <<signal .external_direct.u.req : bit>> <= '0';
    report "req=" & bit'image(<<signal .external_direct.u.req : bit>>);
    wait for 30 ns;
    done <= true;
    wait;
  end process;
  u : entity work.dut port map (clk => clk);
  mon : block
    alias b2 is <<signal .external_direct.u.rec : pair>>.b(2);
    alias ra is <<signal .external_direct.u.rec : pair>>;
    constant d2 : natural := <<constant ^.u.depth : natural>> + 1;
  begin
    process (<<signal .external_direct.u.rec : pair>>) begin
      report "rec.a=" & integer'image(<<signal .external_direct.u.rec : pair>>.a)
        & " ra.b=" & to_hstring(ra.b) & " b2=" & std_logic'image(b2)
        & " count=" & integer'image(<<signal ^.u.count : natural>>) & " d2=" & integer'image(d2);
    end process;
  end block;
  mirror <= <<signal .external_direct.u.count : natural>> * 10;
  process (mirror) begin
    report "mirror=" & integer'image(mirror);
  end process;
end;
