package pkg_scalar is
end package;

package body pkg_scalar is
  function fib(n : natural) return natural is
  begin
    if n < 2 then
      return n;
    end if;
    return fib(n - 1) + fib(n - 2);
  end function;

  function sum_to(n : integer) return integer is
    variable acc : integer := 0;
  begin
    for i in 1 to n loop
      next when i mod 3 = 0;
      acc := acc + i;
      exit when acc > 1000;
    end loop;
    while acc > 10 loop
      acc := acc - 7;
    end loop;
    return acc;
  end function;

  procedure swap(a, b : inout integer) is
    variable t : integer;
  begin
    t := a; a := b; b := t;
  end procedure;

  function classify(x : integer) return string is
  begin
    case x is
      when 0 => return "zero";
      when 1 to 9 => return "small";
      when others => return "big " & integer'image(x);
    end case;
  end function;

  procedure noisy(x : integer) is
    constant r : real := real(x) * 2.5;
    variable t : time := 3 ns;
  begin
    report "noisy " & integer'image(x) severity warning;
    assert x > 0 report "non-positive" severity error;
    assert integer(r) /= 5;
    t := t * 2 + 1 ps;
    if t > 5 ns and not (x = 3) then
      report time'image(t);
    elsif x = 4 or x = 5 then
      null;
    else
      wait for t;
    end if;
  end procedure;
end package body;
