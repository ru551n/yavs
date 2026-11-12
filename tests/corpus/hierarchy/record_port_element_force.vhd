-- difftest: top=record_port_element_force std=2008 ref=hand ignore=order
-- Forces around record ports on elements of arrays of records: a force and release of an element
-- of the actual in the parent, seen through the `in` formal of the lane on it; a force of the
-- actual of one lane's `out` formal (on its own array: that formal keeps its edges, the others
-- on an array nobody forces are aliases); a force of an `in` formal inside an instance of its own
-- entity, which acts on that port only (LRM 10.5.2.2). Same-cycle reports of different processes
-- are unordered. ref=hand: the reference simulators change the actual of the forced `in` port
-- too (`ch(1)` reads C3 from k=3 to k=4); the LRM forces the formal's effective value only, so
-- `ch(1)` keeps its value (`force-in-port-actual` in the discrepancy catalogue).
library ieee;
use ieee.std_logic_1164.all;

package record_port_element_force_pkg is
  type rec_t is record
    v : std_logic;
    d : std_logic_vector(7 downto 0);
  end record;
  type rec_arr_t is array (natural range <>) of rec_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_port_element_force_pkg.all;

entity record_port_element_force_stage is
  generic (IDX : natural);
  port (clk : in std_logic; s : in rec_t; m : out rec_t);
end;
architecture t of record_port_element_force_stage is
begin
  process (clk) begin
    if rising_edge(clk) then
      m.v <= s.v;
      m.d <= std_logic_vector(unsigned(s.d) + 1);
    end if;
  end process;
  mon : process (s) begin
    if now > 0 ns then
      report "lane " & integer'image(IDX) & " s.v=" & to_string(s.v) & " s.d=" & to_hstring(s.d) &
        " s'event=" & boolean'image(s'event);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.record_port_element_force_pkg.all;

entity record_port_element_force_in is
  port (s : in rec_t);
end;
architecture t of record_port_element_force_in is
begin
  process begin
    wait for 22 ns;
    s.d <= force x"c3";
    wait for 20 ns;
    s.d <= release;
    wait;
  end process;
  mon : process (s) begin
    if now > 0 ns then
      report "forced port s.d=" & to_hstring(s.d) & " s'event=" & boolean'image(s'event);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_port_element_force_pkg.all;

entity record_port_element_force is end;
architecture t of record_port_element_force is
  signal clk : std_logic := '0';
  signal ch : rec_arr_t(0 to 3) := (others => ('0', x"00"));
  signal q : rec_arr_t(0 to 2);
  signal qf : rec_arr_t(0 to 0);
begin
  g : for i in 0 to 2 generate
    u : entity work.record_port_element_force_stage generic map (IDX => i)
      port map (clk => clk, s => ch(i), m => q(i));
    ch(i + 1) <= q(i);
  end generate;
  x : entity work.record_port_element_force_stage generic map (IDX => 3)
    port map (clk => clk, s => ch(3), m => qf(0));
  f : entity work.record_port_element_force_in port map (s => ch(1));

  process begin
    wait for 1 ns;
    for k in 1 to 10 loop
      ch(0).d <= std_logic_vector(to_unsigned(16 * k, 8));
      ch(0).v <= not ch(0).v;
      if k = 3 then
        ch(2).d <= force x"5a";
      elsif k = 5 then
        ch(2).d <= release;
      elsif k = 6 then
        qf(0) <= force ('1', x"ee");
      elsif k = 8 then
        qf(0) <= release;
      end if;
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
      report "k=" & integer'image(k) & " ch=" & to_hstring(ch(1).d) & " " & to_hstring(ch(2).d) &
        " " & to_hstring(ch(3).d) & " q=" & to_hstring(q(2).d) & " qf=" & to_hstring(qf(0).d) &
        " v=" & to_string(qf(0).v);
    end loop;
    wait;
  end process;
end;
