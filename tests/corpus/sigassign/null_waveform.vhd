-- difftest: top=null_waveform std=2008
-- A null waveform element in a sequential signal assignment disconnects the driver of a
-- guarded signal from its time on (LRM 10.5.2.2, #352): a bus loses its value (the resolution
-- function sees no driver), a register keeps it. A trailing null on a bus, a register, a
-- vector and an integer with a resolution function, a lone null that preempts a later value,
-- and transport, where the null keeps the value before it.
library ieee; use ieee.std_logic_1164.all;
entity null_waveform is end entity;

architecture test of null_waveform is
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
    b <= '1' after 1 ns, null after 3 ns;
    r <= '1' after 1 ns, null after 3 ns;
    v <= "10" after 1 ns, null after 3 ns;
    n <= 7 after 1 ns, null after 3 ns;
    wait for 2 ns;
    report "2 ns " & std_logic'image(b) & std_logic'image(r) & " " & to_string(v) & " " & integer'image(n);
    wait for 2 ns;
    report "4 ns " & std_logic'image(b) & std_logic'image(r) & " " & to_string(v) & " " & integer'image(n);
    -- A lone null deletes the later value of the driver.
    b <= '0' after 3 ns;
    b <= null after 1 ns;
    wait for 2 ns;
    report "6 ns " & std_logic'image(b);
    wait for 2 ns;
    report "8 ns " & std_logic'image(b);
    -- Transport: the null keeps the value scheduled before it.
    t <= transport '1' after 1 ns;
    t <= transport null after 3 ns;
    wait for 2 ns;
    report "10 ns " & std_logic'image(t);
    wait for 2 ns;
    report "12 ns " & std_logic'image(t);
    wait;
  end process;
end architecture;
