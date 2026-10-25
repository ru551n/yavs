-- difftest: top=guarded_bus_port ref=hand
-- ref=hand: sim1 rejects a disconnection specification of a port, so only sim2 runs it.
-- A guarded (bus) port is a guarded signal: its leaves get the guard kind and the
-- disconnection time of the port's specification (#656). The guarded assignment disconnects
-- 100 ns after the guard goes false, it is not a null transaction on an unguarded signal.
package guarded_bus_pkg is function wired_and (sig : bit_vector) return bit; end;
package body guarded_bus_pkg is
  function wired_and (sig : bit_vector) return bit is
  begin
    if sig'length = 0 then return '1'; end if;
    for i in sig'range loop if sig(i) = '0' then return '0'; end if; end loop;
    return '1';
  end;
end;
use work.guarded_bus_pkg.all;
entity guarded_bus_sub is
  port (signal a : out wired_and bit bus; en : in boolean);
  disconnect a : bit after 100 ns;
end;
architecture x of guarded_bus_sub is
begin
  b : block (en) begin a <= guarded '0'; end block;
end;
use work.guarded_bus_pkg.all;
entity guarded_bus_port is end;
architecture t of guarded_bus_port is
  signal s : wired_and bit bus;
  signal en : boolean := true;
begin
  u : entity work.guarded_bus_sub port map (a => s, en => en);
  process begin
    wait for 10 ns;   report "s1=" & bit'image(s);
    en <= false;
    wait for 50 ns;   report "s2=" & bit'image(s);
    wait for 100 ns;  report "s3=" & bit'image(s);
    wait;
  end process;
end;
