-- difftest: top=mode_view_param_nested std=2019
-- 2019 signal parameters with mode views (6.5.2): a nested element view, its `'converse` in
-- the other side's view, an inout element of a resolved type and a buffer element.
library ieee; use ieee.std_logic_1164.all;
package mode_view_param_nested_pkg is
  type req_t is record valid : bit; data : integer; end record;
  type chan_t is record req : req_t; ready : bit; tag : std_logic; count : natural; end record;
  view req_out of req_t is valid, data : out; end view;
  view chan_m of chan_t is
    req : view req_out;
    ready : in;
    tag : inout;
    count : buffer;
  end view;
  view chan_s of chan_t is
    req : view req_out'converse;
    ready : out;
    tag : inout;
    count : in;
  end view;
  procedure put (signal c : view chan_m; d : integer);
  procedure get (signal c : view chan_s; tag : std_logic);
end package;

package body mode_view_param_nested_pkg is
  procedure put (signal c : view chan_m; d : integer) is
  begin
    c.req.data <= d;
    c.req.valid <= '1';
    c.tag <= 'Z';
    c.count <= c.count + 1;
    wait until c.ready = '1';
    report "put " & integer'image(d) & " tag=" & std_logic'image(c.tag)
      & " count=" & integer'image(c.count);
    c.req.valid <= '0';
  end procedure;

  procedure get (signal c : view chan_s; tag : std_logic) is
  begin
    wait until c.req.valid = '1';
    report "get " & integer'image(c.req.data) & " count=" & integer'image(c.count);
    c.tag <= tag;
    c.ready <= '1', '0' after 1 ns;
    wait until c.req.valid = '0';
    c.tag <= 'Z';
  end procedure;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.mode_view_param_nested_pkg.all;
entity mode_view_param_nested is end;
architecture t of mode_view_param_nested is
  signal ch : chan_t := (('0', 0), '0', 'Z', 0);
begin
  src : process begin
    put(ch, 5);
    wait for 3 ns;
    put(ch, 6);
    wait;
  end process;

  dst : process begin
    get(ch, '1');
    get(ch, '0');
    wait;
  end process;
end;
