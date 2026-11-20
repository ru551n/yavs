-- difftest: top=copy_back_nested_calls std=2008
-- An out actual passed through a temporary (an element of an integer array, a variable of
-- another subtype than the formal's) gets its value back when another argument of the call
-- contains a call that copies its own out parameters back, a native call here (#1179, #1175).
library ieee;
use ieee.std_logic_1164.all, ieee.numeric_std.all;
entity copy_back_nested_calls is end entity;
architecture a of copy_back_nested_calls is
  type iarr is array (0 to 3) of integer;
  procedure set (variable o : out integer; v : integer) is begin o := v; end procedure;
  procedure set2 (v : integer; variable o : out integer) is begin o := v; end procedure;
begin
  process
    variable a : iarr := (others => 0);
    variable q : natural := 0;
    variable i : integer := 1;
    variable u : unsigned(3 downto 0) := "1001";
  begin
    set(a(i), to_integer(u) + 1);
    report "a(1)=" & integer'image(a(1));
    set(q, to_integer(u) + 2);
    report "q=" & integer'image(q);
    set2(to_integer(u) + 3, q);
    report "q=" & integer'image(q);
    set(a(i + 1), to_integer(u(2 downto 0)) + a(i));
    report "a(2)=" & integer'image(a(2));
    assert a(1) = 10 and a(2) = 11 and q = 12 severity failure;
    wait;
  end process;
end architecture;
