-- difftest: top=return_select std=2008
-- A small function that returns one of two values by a branch, called inside a statement: a
-- drive, a variable, an index, an argument of the next call, with an operand that traps.
library ieee;
use ieee.std_logic_1164.all;

entity return_select is
end entity;

architecture a of return_select is
  function bit_of(b : boolean) return std_logic is
  begin
    if b then return '1'; else return '0'; end if;
  end function;

  function pick(b : boolean; x, y : integer) return integer is
  begin
    if b then return x; else return y; end if;
  end function;

  function first_of(b : boolean; x : integer) return integer is
  begin
    if b then return x; else return 7; end if;
  end function;

  function chain(b, c : boolean) return integer is
  begin
    if b then
      if c then return 1; else return 2; end if;
    else
      if c then return 3; else return 4; end if;
    end if;
  end function;

  function name_of(b : boolean) return string is
  begin
    if b then return "yes"; else return "no"; end if;
  end function;

  signal s, t : std_logic := 'U';
  signal v : std_logic_vector(3 downto 0) := "0000";
  signal n : integer := 0;
begin
  process
    variable i : integer := 0;
    variable a : integer_vector(0 to 3) := (10, 11, 12, 13);
  begin
    for k in 0 to 5 loop
      s <= bit_of(k mod 2 = 0);
      t <= bit_of(k > 2);
      v <= bit_of(k = 1) & bit_of(k = 2) & bit_of(k = 3) & bit_of(k = 4);
      n <= pick(k > 3, k, -k);
      i := a(pick(k mod 2 = 0, 1, 3));
      wait for 1 ns;
      report to_string(s) & to_string(t) & " " & to_string(v) & " " & integer'image(n) & " "
        & integer'image(i) & " " & integer'image(first_of(k = 2, i))
        & " " & integer'image(chain(k < 3, k mod 2 = 1)) & " " & name_of(k = 0);
    end loop;
    wait;
  end process;
end architecture;
