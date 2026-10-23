-- difftest: top=case_aggregate_choice std=2008
-- Aggregates with an others choice as case choices take the subtype of the case expression
-- (a signal, a variable and a constrained port subtype), also in a selected assignment.
library ieee;
use ieee.std_logic_1164.all;

entity case_aggregate_choice is end entity;

architecture a of case_aggregate_choice is
  signal sel : std_logic_vector(5 downto 0) := "000100";
  signal one : std_logic_vector(2 downto 0) := "000";
  signal res : integer := -1;
begin
  with sel select res <=
    0 when (0 => '1', others => '0'),
    2 when (2 => '1', others => '0'),
    9 when others;

  process
    variable v : bit_vector(1 to 4) := "0100";
    function pick (s : std_logic_vector(5 downto 0)) return integer is
    begin
      case s is
        when (0 => '1', others => '0') => return 0;
        when (2 => '1', others => '0') => return 2;
        when others => return 9;
      end case;
    end function;
  begin
    wait for 1 ns;
    assert res = 2 report "selected assignment" severity failure;
    assert pick(sel) = 2 report "case in a function" severity failure;
    assert pick("000001") = 0 report "first choice" severity failure;
    assert pick("111111") = 9 report "others" severity failure;
    case v is
      when (1 => '1', others => '0') => report "wrong" severity failure;
      when (2 => '1', others => '0') => report "variable ok";
      when others => report "wrong" severity failure;
    end case;
    wait;
  end process;
end architecture;
