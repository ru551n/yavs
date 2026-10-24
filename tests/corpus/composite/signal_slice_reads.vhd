-- difftest: top=signal_slice_reads std=2008
-- Slices of a signal read as values: logic and other vectors, both directions, a port, a null
-- slice, bounds that depend on a loop variable, a slice of a slice, and a slice in an
-- expression next to the whole signal.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity signal_slice_reads_m is
  port (
    d  : in  std_logic_vector(15 downto 0);
    up : in  std_logic_vector(0 to 7);
    q  : out std_logic_vector(7 downto 0)
  );
end entity;

architecture a of signal_slice_reads_m is
begin
  q <= d(11 downto 4) xor (up(2 to 5) & up(0 to 3));
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity signal_slice_reads is
end entity;

architecture t of signal_slice_reads is
  type int_vec is array (natural range <>) of integer;
  signal v   : std_logic_vector(15 downto 8) := x"a5";
  signal w   : std_logic_vector(0 to 7) := "10010110";
  signal b   : bit_vector(3 downto 0) := "1010";
  signal iv  : int_vec(2 to 6) := (10, 20, 30, 40, 50);
  signal u   : unsigned(7 downto 0) := x"3c";
  signal dd  : std_logic_vector(15 downto 0) := x"1234";
  signal uu  : std_logic_vector(0 to 7) := x"0f";
  signal qq  : std_logic_vector(7 downto 0);
begin
  m : entity work.signal_slice_reads_m port map (d => dd, up => uu, q => qq);

  process
    variable n : natural;
    variable s : integer;
  begin
    wait for 1 ns;
    report "a " & to_string(v(11 downto 9)) & " " & to_string(w(2 to 5)) & " " & to_string(b(2 downto 1))
      & " " & to_string(u(7 downto 4)) & " " & to_string(unsigned(u(3 downto 0)) + 1);
    s := 0;
    for i in iv(3 to 5)'range loop
      s := s + iv(i);
    end loop;
    n := 3;
    report "b " & integer'image(iv(2 to 4)(3)) & " " & integer'image(s) & " " & to_string(v(8 + n downto 8))
      & " " & to_string(w(n to n + 2)(n + 1 to n + 2));
    -- null slices
    report "c [" & to_string(v(9 downto 10)) & "] " & integer'image(w(5 to 4)'length) & " "
      & integer'image(iv(4 to 3)'length);
    -- loop-dependent bounds
    for i in 0 to 3 loop
      report "d" & integer'image(i) & " " & to_string(v(8 + i + 3 downto 8 + i)) & " " & to_string(w(i to i + 3));
    end loop;
    wait for 1 ns;
    report "e " & to_string(qq);
    dd <= x"f00f";
    uu <= x"c3";
    wait for 1 ns;
    report "f " & to_string(qq);
    v <= x"3c";
    wait for 1 ns;
    assert v(15 downto 12) = "0011" report "v hi" severity failure;
    assert v(11 downto 8) = "1100" report "v lo" severity failure;
    wait;
  end process;
end architecture;
