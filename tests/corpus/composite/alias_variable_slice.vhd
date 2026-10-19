-- difftest: top=alias_variable_slice std=2008
-- An alias of a slice of a variable in a process (#296): logic vector, bit vector and
-- integer array, ascending and descending, written through the variable afterwards.
library ieee;
use ieee.std_logic_1164.all;
entity alias_variable_slice is end entity;
architecture t of alias_variable_slice is
  type iv is array (natural range <>) of integer;
begin
  process
    constant PARAM_LEN : integer := 8;
    constant ID_LEN : integer := 4;
    variable RxParam : std_logic_vector(PARAM_LEN-1 downto 0) := x"A5";
    alias RxID : std_logic_vector(ID_LEN-1 downto 0) is RxParam(PARAM_LEN-1 downto PARAM_LEN-ID_LEN);
    alias e5 : std_logic is RxParam(5);
    variable b : bit_vector(0 to 7) := "11000101";
    alias bl : bit_vector(0 to 2) is b(1 to 3);
    variable i : iv(1 to 5) := (10, 20, 30, 40, 50);
    alias ia : iv(0 to 1) is i(2 to 3);
  begin
    assert RxID'length = 4 severity failure;
    assert RxID = x"A" severity failure;
    RxParam := x"3C";
    assert RxID = x"3" severity failure;
    assert RxID(0) = '1' severity failure;
    RxID := x"F";
    assert RxParam = x"FC" severity failure;
    RxID(1) := '0';
    assert RxParam = x"DC" severity failure;
    assert bl = "100" severity failure;
    b(2) := '1';
    assert bl = "110" severity failure;
    assert ia(0) = 20 and ia(1) = 30 severity failure;
    ia(1) := 99;
    assert i(3) = 99 severity failure;
    e5 := '1';
    assert RxParam = x"FC" severity failure;
    report "id " & integer'image(RxID'length);
    wait;
  end process;
end architecture;
