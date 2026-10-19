-- difftest: top=mode_view_param_array std=2019
-- 2019 signal parameters with an array mode view (`view (v) of arr_t`, 6.5.2): the procedure
-- indexes the array with a loop variable and drives one element kind of every record; two
-- processes pass disjoint slices of one array signal.
package mode_view_param_array_pkg is
  type bus_t is record valid : bit; data : integer; ready : bit; end record;
  view bus_m of bus_t is valid, data : out; ready : in; end view;
  alias bus_s is bus_m'converse;
  type buses_t is array (natural range <>) of bus_t;
  procedure answer (signal bs : view (bus_s) of buses_t);
end package;

package body mode_view_param_array_pkg is
  procedure answer (signal bs : view (bus_s) of buses_t) is
    variable seen : natural := 0;
  begin
    while seen < bs'length loop
      wait until bs'event;
      for i in bs'range loop
        if bs(i).valid = '1' and bs(i).ready = '0' then
          report "answer " & integer'image(i) & " data=" & integer'image(bs(i).data);
          bs(i).ready <= '1';
          seen := seen + 1;
        end if;
      end loop;
    end loop;
  end procedure;
end package body;

use work.mode_view_param_array_pkg.all;
entity mode_view_param_array is end;
architecture t of mode_view_param_array is
  signal buses : buses_t(1 to 4);
begin
  low : process begin
    answer(buses(1 to 2));
    report "low answered";
    wait;
  end process;

  high : process begin
    answer(buses(3 to 4));
    report "high answered";
    wait;
  end process;

  g : for i in buses'range generate
    process begin
      wait for i * 2 ns;
      buses(i).data <= 100 + i;
      buses(i).valid <= '1';
      wait until buses(i).ready = '1';
      report "ready " & integer'image(i);
      wait;
    end process;
  end generate;
end;
