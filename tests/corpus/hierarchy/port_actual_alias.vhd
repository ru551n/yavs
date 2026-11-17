-- difftest: top=tb_port_actual_alias std=2008
-- An expression actual of an `in` port that reads a signal through an alias (of a record field, of
-- a slice) follows the signal: its equivalent process waits on what the alias names (LRM 6.5.6.3,
-- 10.2). It had waited on nothing and kept its first value (found by the cycle mode fuzzer). A
-- `wait until` on an alias waits on it too.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity sub_paa is
  port (b : in unsigned(15 downto 0); c : in std_ulogic_vector(3 downto 0));
end entity;
architecture a of sub_paa is
begin
  process (b, c)
  begin
    report "b=" & to_hstring(b) & " c=" & to_string(c);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb_port_actual_alias is end entity;
architecture a of tb_port_actual_alias is
  type rec_t is record
    f0 : unsigned(1 to 64);
    f1 : std_ulogic;
  end record;
  signal r : rec_t := (f0 => (others => '0'), f1 => '0');
  signal v : std_ulogic_vector(7 downto 0) := x"00";
  alias rf is r.f0;
  alias vh : std_ulogic_vector(0 to 3) is v(7 downto 4);
begin
  u : entity work.sub_paa port map (b => resize(rf(36 to 47), 16), c => vh xor "0001");
  process
  begin
    wait for 1 ns;
    r.f0 <= x"F05E65A48F2C9E44";
    wait for 1 ns;
    v <= x"A5";
    wait for 1 ns;
    r.f0 <= x"0123456789ABCDEF";
    wait;
  end process;
  waiter : process
  begin
    wait until vh(0) = '1';
    report "vh(0) = '1' at v=" & to_hstring(v);
    wait;
  end process;
end architecture;
