-- difftest: top=mode_view_nested std=2019 ignore=order
-- ignore=order: processes report in one cycle, ordered by yavs's rule (#252), which the
-- reference simulator does not follow here, so each process's reports are compared.
-- 2019 mode views (6.5.2): a nested element view, an array of records (`view (v) of`), a
-- view port passed down to a child with the same view and an inout element.
library ieee; use ieee.std_logic_1164.all;
package mode_view_nested_pkg is
  type req_t is record valid : bit; data : integer; end record;
  type chan_t is record req : req_t; ready : bit; tag : std_logic; end record;
  view req_out of req_t is valid, data : out; end view;
  view chan_m of chan_t is
    req : view req_out;
    ready : in;
    tag : inout;
  end view;
  alias chan_s is chan_m'converse;
  type chans_t is array (0 to 1) of chan_t;
end package;

library ieee; use ieee.std_logic_1164.all;
use work.mode_view_nested_pkg.all;
entity mode_view_nested_leaf is generic (id : integer); port (c : view chan_m); end;
architecture a of mode_view_nested_leaf is begin
  process begin
    c.tag <= 'Z';
    c.req.data <= 10 * id;
    c.req.valid <= '1';
    wait until c.ready = '1';
    report "leaf " & integer'image(id) & " ready, tag=" & std_logic'image(c.tag);
    wait;
  end process;
end;

use work.mode_view_nested_pkg.all;
entity mode_view_nested_mid is port (c : view chan_m); end;
architecture a of mode_view_nested_mid is begin
  leaf : entity work.mode_view_nested_leaf generic map (id => 1) port map (c => c);
end;

library ieee; use ieee.std_logic_1164.all;
use work.mode_view_nested_pkg.all;
entity mode_view_nested_sink is port (cs : view (chan_s) of chans_t); end;
architecture a of mode_view_nested_sink is begin
  g : for i in cs'range generate
    process begin
      wait until cs(i).req.valid = '1';
      report "sink " & integer'image(i) & " data=" & integer'image(cs(i).req.data);
      cs(i).tag <= '1';
      cs(i).ready <= '1' after 1 ns;
      wait;
    end process;
  end generate;
end;

use work.mode_view_nested_pkg.all;
entity mode_view_nested is end;
architecture t of mode_view_nested is
  signal chans : chans_t;
begin
  mid : entity work.mode_view_nested_mid port map (c => chans(0));
  leaf : entity work.mode_view_nested_leaf generic map (id => 2) port map (c => chans(1));
  sink : entity work.mode_view_nested_sink port map (cs => chans);
end;
