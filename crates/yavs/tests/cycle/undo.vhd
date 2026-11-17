-- A constant whose value needs an impure call: elaboration cannot evaluate D (it reads a shared
-- variable), so D is left to the run (W2118). The value pass of cycle mode tries D again for C,
-- makes the impure call and undoes it, so the run makes it once, as the normal mode does
-- (D = 1 + 3, and the next call returns 2). Needs --relaxed (a shared variable of a plain type).
package cycle_undo_pk is
  type counter_t is protected
    impure function nxt return integer;
  end protected;
  impure function bump return integer;
end package;
package body cycle_undo_pk is
  type counter_t is protected body
    variable n : integer := 0;
    impure function nxt return integer is begin n := n + 1; return n; end function;
  end protected body;
  shared variable cnt : counter_t;
  impure function bump return integer is begin return cnt.nxt; end function;
end package body;
library ieee; use ieee.std_logic_1164.all; use work.cycle_undo_pk.all;
entity cycle_undo is end;
architecture a of cycle_undo is
  shared variable sv : integer := 3;
  constant D : integer := bump + sv;
  -- No impure call of its own: the value pass evaluates it, and D on the way.
  constant C : integer := D + 10;
  signal clk : std_ulogic := '0';
  signal q : integer := 0;
begin
  clk <= not clk after 5 ns when now < 20 ns else clk;
  m : process (clk) begin
    if rising_edge(clk) then q <= C; end if;
  end process;
  chk : process begin
    wait for 30 ns;
    report "D=" & integer'image(D) & " C=" & integer'image(C) & " q=" & integer'image(q) &
      " next=" & integer'image(bump);
    wait;
  end process;
end;
