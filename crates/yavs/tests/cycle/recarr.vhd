-- Arrays of records in cycle mode against the normal mode on 0/1 inputs, for a width that the test
-- fills in: signals (a bus fabric of one process per port, an index chosen at run time, slices, an
-- array of records in a `to` range) and a variable of a clocked process written at a run-time index.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_recarr_pkg is
  type req_t is record
    addr : std_ulogic_vector(W - 1 downto 0);
    stb : std_ulogic;
    tag : std_ulogic_vector(2 downto 0);
  end record;
  type req_arr_t is array (natural range <>) of req_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_recarr_pkg.all;
entity cycle_recarr is
end entity;
architecture a of cycle_recarr is
  signal clk : std_ulogic := '0';
  signal main : req_t := ((others => '0'), '0', "000");
  signal k : unsigned(1 downto 0) := "00";
  signal dev : req_arr_t(0 to 3);
  signal up : req_arr_t(5 downto 2);
  signal pick, last : req_t;
  signal mid : req_arr_t(0 to 1);
  signal regs : req_arr_t(0 to 3);
begin
  ports : for i in 0 to 3 generate
    port_p : process (main)
    begin
      dev(i) <= main;
      if main.tag(1 downto 0) = std_ulogic_vector(to_unsigned(i, 2)) then
        dev(i).stb <= main.stb;
      else
        dev(i).stb <= '0';
      end if;
    end process;
  end generate;

  ups : process (all)
  begin
    for i in 2 to 5 loop
      up(i) <= dev(i - 2);
      up(i).tag <= dev(5 - i).tag xor dev(i - 2).tag;
    end loop;
  end process;

  sel : process (all)
  begin
    pick <= dev(to_integer(k));
    pick.addr <= dev(to_integer(k)).addr xor up(to_integer(k) + 2).addr;
    mid <= dev(1 to 2);
  end process;

  reg : process (clk)
    variable v : req_arr_t(0 to 3);
    variable started : boolean := false;
  begin
    if rising_edge(clk) then
      if not started then
        v := up;
        started := true;
      end if;
      v(to_integer(k)) := main;
      v(1).stb := not v(1).stb;
      v(3).addr := v(2).addr xor v(0).addr;
      regs <= v;
      last <= v(to_integer(not k));
    end if;
  end process;

  main_p : process
    variable x : unsigned(63 downto 0) := x"9E3779B97F4A7C15";
    variable va : std_ulogic_vector(W - 1 downto 0);
  begin
    wait for 1 ns;
    for c in 1 to 16 loop
      for j in 0 to W - 1 loop
        va(j) := x(j mod 64) xor x((j * 7 + 3) mod 64);
      end loop;
      main <= (va, x(5), std_ulogic_vector(x(9 downto 7)));
      k <= x(12 downto 11);
      x := (x(62 downto 0) & '0') xor (x(63 downto 0) and x"000000000000001B" and (63 downto 0 => x(63)));
      x := x + 16#5851F42D#;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "c=" & integer'image(c) & " d0=" & to_hstring(dev(0).addr) & std_ulogic'image(dev(0).stb)
        & std_ulogic'image(dev(3).stb) & " up=" & to_hstring(up(4).addr) & to_string(up(2).tag)
        & to_string(up(5).tag) & " pick=" & to_hstring(pick.addr) & std_ulogic'image(pick.stb)
        & to_string(pick.tag) & " mid=" & std_ulogic'image(mid(0).stb) & to_string(mid(1).tag)
        & " regs=" & to_hstring(regs(0).addr) & to_hstring(regs(3).addr) & std_ulogic'image(regs(1).stb)
        & to_string(regs(2).tag) & " last=" & to_hstring(last.addr) & std_ulogic'image(last.stb);
    end loop;
    wait;
  end process;
end architecture;
