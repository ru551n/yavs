-- difftest: top=sigrec_array_fn_width std=2008
-- An array of records whose field width comes from a function call, driven by element and by
-- field in a process that waits, and read whole (#677): the optimiser keeps the descriptor of the
-- first activation in the frame, and the operands of the scatter and gather follow.
package fw_fn is
  function get_width return natural;
end package;
package body fw_fn is
  function get_width return natural is
  begin
    return 5;
  end;
end package body;

use work.fw_fn.all;
package fw_pkg is
  type t_record is record
    descriptor : bit_vector(get_width - 1 downto 0);
    k : integer;
  end record;
  type t_array is array (natural range <>) of t_record;
  function img (a : t_array) return string;
end package;
package body fw_pkg is
  function img (a : t_array) return string is
    variable s : string(1 to 200);
    variable n : natural := 0;
  begin
    for i in a'range loop
      s(n + 1 to n + 5) := to_string(a(i).descriptor);
      s(n + 6 to n + 6 + integer'image(a(i).k)'length) := "/" & integer'image(a(i).k);
      n := n + 6 + integer'image(a(i).k)'length;
      s(n + 1) := ' ';
      n := n + 1;
    end loop;
    return s(1 to n);
  end;
end package body;

use work.fw_pkg.all;
entity sigrec_array_fn_width is end;
architecture a of sigrec_array_fn_width is
  signal chans : t_array(0 to 2);
begin
  stim : process
    variable v : t_array(0 to 2);
  begin
    wait for 1 ns;
    chans(1) <= (descriptor => "10110", k => 7);
    wait for 1 ns;
    chans(0).descriptor(0) <= '1';
    wait for 1 ns;
    report "first " & img(chans);
    for round in 1 to 3 loop
      wait for 1 ns;
      chans(1) <= (descriptor => "10110", k => round);
      chans(round - 1).descriptor(0) <= '1';
      wait for 1 ns;
      v := chans;
      report integer'image(round) & " " & img(v);
      for i in chans'range loop
        chans(i) <= (descriptor => "01010", k => 10 * round + i);
      end loop;
      wait for 1 ns;
      report integer'image(round) & " " & img(chans);
    end loop;
    wait;
  end process;
end architecture;
