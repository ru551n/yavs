-- difftest: top=null_waveform_targets std=2008 ref=hand
-- A null waveform element on an aggregate target, a record and a signal parameter of a guarded
-- actual (LRM 10.5.2.2, 4.2.2.3). ref=hand: sim1 rejects a null on a parameter and sim2 does not
-- run null waveform elements in this form; the expected values follow LRM 14.7.3.
library ieee; use ieee.std_logic_1164.all;
entity null_waveform_targets is end;
architecture a of null_waveform_targets is
  type rec is record p, q : std_ulogic; end record;
  type rv is array (natural range <>) of rec;
  function res (v : rv) return rec is
    variable r : rec := ('Z', 'Z');
  begin
    for i in v'range loop r := v(i); end loop; return r;
  end;
  subtype rrec is res rec;
  type r2 is record p, q : resolved std_ulogic; end record;
  signal g : r2 bus;
  signal h : r2 bus;
  signal p, x, y : std_logic bus;
  signal pv : std_logic_vector(1 downto 0) bus;
  procedure d (signal s : out std_logic) is begin s <= '1', null after 2 ns; end;
  procedure dv (signal s : out std_logic_vector) is begin s <= "10", null after 2 ns; end;
  procedure dr (signal s : out r2) is begin s <= ('1', '0'), null after 2 ns; end;
begin
  process begin
    d(p); dv(pv); dr(h);
    g <= ('1', '0'), null after 2 ns;
    wait for 1 ns;
    report to_string(p) & to_string(pv) & to_string(g.p) & to_string(g.q) & to_string(h.p);
    wait for 2 ns;
    report to_string(p) & to_string(pv) & to_string(g.p) & to_string(g.q) & to_string(h.p);
    (x, y) <= std_logic_vector'("10"), null after 2 ns;
    wait for 1 ns;
    report to_string(x) & to_string(y);
    wait for 2 ns;
    report to_string(x) & to_string(y);
    (x, y) <= null after 1 ns, std_logic_vector'("11") after 2 ns;
    wait for 3 ns;
    report to_string(x) & to_string(y);
    wait;
  end process;
end;
