-- difftest: top=ext_force_out_port std=2008
-- A force through an external name with no mode written on the output port of a child entity
-- is a force out: the connected net changes (scalar and vector), and the release restores it.
library ieee;
use ieee.std_logic_1164.all;
entity slv is
  port (q : out std_logic; qv : out std_logic_vector(3 downto 0));
end;
architecture a of slv is
begin
  q <= '0';
  qv <= "0000";
end;
library ieee;
use ieee.std_logic_1164.all;
entity ext_force_out_port is end;
architecture a of ext_force_out_port is
  signal q : std_logic;
  signal qv : std_logic_vector(3 downto 0);
  signal ev : integer := 0;
begin
  s : entity work.slv port map (q, qv);
  process
  begin
    wait on q, qv;
    if q'event and qv'event then ev <= ev + 100; elsif q'event then ev <= ev + 1; elsif qv'event then ev <= ev + 10; end if;
  end process;
  process
    alias a_q is << signal .ext_force_out_port.s.q : std_logic >>;
    alias a_qv is << signal .ext_force_out_port.s.qv : std_logic_vector(3 downto 0) >>;
  begin
    wait for 10 ns;
    a_q <= force not a_q;
    wait for 1 ns;
    report "q=" & std_logic'image(q) & " ev=" & integer'image(ev);
    a_qv <= force not a_qv;
    wait for 1 ns;
    report "qv=" & to_string(qv) & " ev=" & integer'image(ev);
    a_q <= release; a_qv <= release;
    wait for 1 ns;
    report "rel q=" & std_logic'image(q) & " qv=" & to_string(qv) & " ev=" & integer'image(ev);
    wait;
  end process;
end;
