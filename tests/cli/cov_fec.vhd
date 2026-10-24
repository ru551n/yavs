-- Condition and expression coverage (crates/yavs/tests/coverage_fec.rs): the rows of every
-- condition and expression below are pinned there.
library ieee;
use ieee.std_logic_1164.all;

entity cov_fec is generic (gt : boolean := true);
end entity;

architecture sim of cov_fec is
  signal a, b, c : boolean := false;
  signal x, y    : std_logic := '0';
  signal q       : std_logic;
  signal r       : boolean; constant kf : boolean := false;
  type lv is array (0 to 3) of std_logic;
  constant xs : lv := ('0', 'H', 'X', '1');
  constant ys : lv := ('1', 'L', '1', 'U');
  function both(p, q : boolean) return boolean is begin if p and q then return true; end if; return false; end function;
begin
  -- Concurrent: a condition on booleans and an expression on std_logic.
  r <= true when a and b else false;
  q <= x xor y;

  stim : process
    variable n, calls, k : integer := 0;
    variable v           : boolean;
    impure function tick(t : boolean) return boolean is
    begin
      calls := calls + 1;
      return t;
    end function;
  begin
    -- (a, b, c) over all eight combinations.
    for i in 0 to 7 loop
      a <= (i / 4) mod 2 = 1;
      b <= (i / 2) mod 2 = 1;
      c <= i mod 2 = 1;
      wait for 1 ns;
      if a and (b or c) then
        n := n + 1;
      end if;
      v := not (a xor b) or c;
      -- Function calls are inputs: `tick(b)` is not called when `tick(a)` is false.
      if tick(a) and tick(b) then
        n := n + 10;
      end if;
    end loop;
    -- std_logic inputs: 'H' is 1, 'L' is 0, 'X' and 'U' count nothing.
    for i in 0 to 3 loop
      x <= xs(i);
      y <= ys(i);
      wait for 1 ns;
      if x and y then
        n := n + 100;
      end if;
    end loop;
    -- Loop conditions.
    while k < 4 and not (n > 1000) loop
      k := k + 1;
      next when k = 1 or k = 3;
      exit when k = 2 and n > 1000;
    end loop;
    -- One input: no rows.
    if a then
      n := n + 1;
    end if;
    -- A row that is never covered: `c` is never true while `a` is false and `b` true.
    if (not a and b) and c then
      n := n + 1000;
    end if;
    -- Inputs constant after elaboration (generic, constant, literal) have no rows; a condition
    -- with fewer than two other inputs has no item at all.
    if (a and gt) or (b and not kf) then
      n := n + 1;
    end if;
    if a and gt then
      n := n + 1;
    end if;
    if gt or kf then
      n := n + 1;
    end if;
    if b and true and (a or kf) then
      n := n + 1;
    end if;
    -- A constant first input starts the state; subprogram formals are not constants.
    if gt and a and b then
      n := n + 1;
    end if;
    for i in 0 to 3 loop
      v := both(i mod 2 = 1, i / 2 = 1);
    end loop;
    report "n=" & integer'image(n) & " calls=" & integer'image(calls);
    wait;
  end process;
end architecture;
