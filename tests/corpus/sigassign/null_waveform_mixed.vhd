-- difftest: top=null_waveform_mixed std=2008 ref=hand
-- ref=hand: sim1 drops the first element of '1', null after 2 ns and the null of
-- '1' after 1 ns, null after 3 ns, '0' after 5 ns (#352); .expected is the LRM 10.5.2.2 result.
-- A null in the middle of a waveform, after a zero-delay value, and null first.
library ieee; use ieee.std_logic_1164.all;
entity null_waveform_mixed is end entity;

architecture test of null_waveform_mixed is
  type ints is array (natural range <>) of integer;
  function sum(v : ints) return integer is
    variable r : integer := 100;
  begin
    for i in v'range loop r := r + v(i); end loop;
    return r;
  end function;
  subtype rint is sum integer;
  signal b : std_logic bus;
  signal r : std_logic register := '0';
  signal v : std_logic_vector(1 downto 0) bus;
  signal n : rint bus;
  signal t : std_logic bus;
begin
  process
  begin
    b <= '1', null after 2 ns, '0' after 4 ns;
    r <= '1', null after 2 ns, '0' after 4 ns;
    v <= "10", null after 2 ns, "01" after 4 ns;
    n <= 7, null after 2 ns, 9 after 4 ns;
    wait for 1 ns;
    report "1 ns " & std_logic'image(b) & std_logic'image(r) & " " & to_string(v) & " " & integer'image(n);
    wait for 2 ns;
    report "3 ns " & std_logic'image(b) & std_logic'image(r) & " " & to_string(v) & " " & integer'image(n);
    wait for 2 ns;
    report "5 ns " & std_logic'image(b) & std_logic'image(r) & " " & to_string(v) & " " & integer'image(n);
    -- A null first replaces the value scheduled before it.
    b <= '1' after 5 ns;
    b <= null after 1 ns, '1' after 3 ns;
    wait for 2 ns;
    report "7 ns " & std_logic'image(b);
    wait for 2 ns;
    report "9 ns " & std_logic'image(b);
    -- A null at once takes effect in the next delta cycle.
    b <= null;
    wait for 0 ns;
    report "lone " & std_logic'image(b);
    -- Transport: the first element deletes the later value, the null follows it.
    t <= transport '1' after 3 ns;
    t <= transport '0' after 1 ns, null after 2 ns;
    wait for 1 ns;
    report "t 1 ns " & std_logic'image(t);
    wait for 1 ns;
    report "t 2 ns " & std_logic'image(t);
    wait for 2 ns;
    report "t 4 ns " & std_logic'image(t);
    wait;
  end process;
end architecture;
