-- yavs-test: std=2019
-- expect: none
-- LRM 2019 9.1, 10.6.2.1, 10.14
library ieee;
use ieee.std_logic_1164.all;
entity e is
end entity;
architecture a of e is
  constant g : boolean := true;
  signal s : std_logic_vector(3 downto 0) := (others => '0') when g else "1010";
  function f(x : integer) return integer is
  begin
    return 1 when x > 0 else -1 when x < 0;
    return 0;
  end function;
  procedure p(b : boolean; i : integer) is
  begin
  end procedure;
begin
  process
    variable v : integer := 3 + (1 when g else 2);
    variable w : bit_vector(3 downto 0);
    variable l : std_ulogic := '1';
  begin
    v := 1 when l else 2;
    v := unaffected when g else 5;
    v := 7 when not g;
    v := unaffected;
    w := (others => '1') when g else unaffected;
    s <= force (others => '1') when g else unaffected;
    p(true when g else false, (2 when g else 3));
    if (l when g else '0') then
      v := f(v);
    end if;
    wait;
  end process;
end architecture;
