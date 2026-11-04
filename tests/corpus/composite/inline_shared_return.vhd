-- difftest: top=inline_shared_return std=2008
-- Inlined functions with several branches into one `return` (#1094): a short circuit `or` or `and`
-- leaves two branches to the same return, one of which may read a constant defined in the other.
-- One read of an unset register returned 0 for '1' in `to_int`. Expected values are by hand: the
-- text of each report line is what the function body says for each input.
library ieee;
use ieee.std_logic_1164.all;

entity inline_shared_return is
end entity;

architecture a of inline_shared_return is
  type color is (red, green, blue);
  type int_vec is array (natural range <>) of integer;

  constant inputs : std_ulogic_vector(0 to 5) := "01LHXZ";

  -- The shape of the bug, and siblings with more arms and other values.
  function to_int(v : std_ulogic) return integer is
  begin
    if v = '1' or v = 'H' then
      return 1;
    end if;
    return 0;
  end function;

  function three_arms(v : std_ulogic) return integer is
  begin
    if v = '1' or v = 'H' or v = 'X' then
      return 7;
    end if;
    return 2;
  end function;

  function four_arms(v : std_ulogic) return integer is
  begin
    if v = '0' or v = 'L' or v = 'X' or v = 'Z' then
      return -5;
    end if;
    return 31;
  end function;

  function shared_zero(v : std_ulogic) return integer is
  begin
    if v = '1' or v = 'H' then
      return 0;
    end if;
    return 31;
  end function;

  function extremes(v : std_ulogic) return integer is
  begin
    if v = '1' or v = 'H' then
      return integer'high;
    end if;
    return integer'low;
  end function;

  function two_levels(v : std_ulogic) return integer is
  begin
    if v = '1' or v = 'H' then
      return 1;
    elsif v = '0' or v = 'L' then
      return 2;
    end if;
    return 3;
  end function;

  function both(a, b : boolean) return integer is
  begin
    if a and b then
      return 9;
    end if;
    return 31;
  end function;

  function either_or_value(a, b : boolean; x : integer) return integer is
  begin
    if a or b then
      return x;
    end if;
    return 5;
  end function;

  function to_bool(v : std_ulogic) return boolean is
  begin
    if v = '1' or v = 'H' then
      return true;
    end if;
    return false;
  end function;

  function to_logic(v : std_ulogic) return std_ulogic is
  begin
    if v = '1' or v = 'H' then
      return '1';
    end if;
    return '0';
  end function;

  function to_color(v : std_ulogic) return color is
  begin
    if v = '1' or v = 'H' then
      return green;
    end if;
    return blue;
  end function;

  -- Inlined into a function with a loop, as the bug was found.
  function to_ints(v : std_ulogic_vector) return int_vec is
    variable r : int_vec(v'range);
  begin
    for i in v'range loop
      r(i) := to_int(v(i));
    end loop;
    return r;
  end function;

  signal s : std_ulogic := 'U';
begin
  -- Inlined into a process, once per function, on every input.
  process
    variable x : int_vec(0 to 5);
    variable sum : integer := 0;
  begin
    for k in inputs'range loop
      report integer'image(k) & " "
        & integer'image(to_int(inputs(k))) & " "
        & integer'image(three_arms(inputs(k))) & " "
        & integer'image(four_arms(inputs(k))) & " "
        & integer'image(shared_zero(inputs(k))) & " "
        & integer'image(extremes(inputs(k))) & " "
        & integer'image(two_levels(inputs(k))) & " "
        & boolean'image(to_bool(inputs(k))) & " "
        & std_ulogic'image(to_logic(inputs(k))) & " "
        & color'image(to_color(inputs(k)));
      sum := sum + to_int(inputs(k));
      s <= to_logic(inputs(k));
      wait for 1 ns;
      report "drive " & std_ulogic'image(s);
    end loop;
    report "sum " & integer'image(sum);
    for a in boolean loop
      for b in boolean loop
        report boolean'image(a) & " " & boolean'image(b) & " "
          & integer'image(both(a, b)) & " " & integer'image(either_or_value(a, b, sum));
      end loop;
    end loop;
    x := to_ints(inputs);
    report integer'image(x(0)) & integer'image(x(1)) & integer'image(x(2))
      & integer'image(x(3)) & integer'image(x(4)) & integer'image(x(5));
    report integer'image(to_ints(inputs)(1));
    wait;
  end process;
end architecture;
