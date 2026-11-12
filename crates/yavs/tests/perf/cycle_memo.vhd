-- A function of eight local vectors: the optimiser asks for each of their copies whether its
-- block is on a cycle, and the second and later questions about the same block come from the
-- pass's memo (`opt_cycle_memo`), not from another walk of the function.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_memo is end;
architecture t of cycle_memo is
  function mix(x : std_logic_vector(7 downto 0)) return integer is
    variable v0 : std_logic_vector(7 downto 0) := (others => '0');
    variable v1 : std_logic_vector(7 downto 0) := (others => '0');
    variable v2 : std_logic_vector(7 downto 0) := (others => '0');
    variable v3 : std_logic_vector(7 downto 0) := (others => '0');
    variable v4 : std_logic_vector(7 downto 0) := (others => '0');
    variable v5 : std_logic_vector(7 downto 0) := (others => '0');
    variable v6 : std_logic_vector(7 downto 0) := (others => '0');
    variable v7 : std_logic_vector(7 downto 0) := (others => '0');
  begin
    v0 := x;
    v1 := v0 xor x"0f";
    v2 := v1 xor x"33";
    v3 := v2 xor x"55";
    v4 := v3 xor x"a5";
    v5 := v4 xor x"3c";
    v6 := v5 xor x"c3";
    v7 := v6 xor x"81";
    return to_integer(unsigned(v7));
  end function;
begin
  process
    variable s : integer := 0;
  begin
    for i in 0 to 20 loop
      s := (s + mix(std_logic_vector(to_unsigned(i, 8)))) mod 1000;
    end loop;
    report "m=" & integer'image(s);
    wait;
  end process;
end;
