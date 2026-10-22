-- difftest: top=user_reduce_idiom std=2008
-- User functions that fold a logic vector with one gate (the `or_reduce` idiom) against the
-- same fold written inline, over every vector of up to three of the nine values, ascending and
-- descending, in a package and in an architecture; look-alikes with another start value, another
-- operand order or a user gate stay what their bodies say.
library ieee; use ieee.std_logic_1164.all;
package red_pkg is
  function or_f(d : std_ulogic_vector) return std_ulogic;
  function and_f(d : std_logic_vector) return std_ulogic;
  function xor_f(d : std_ulogic_vector) return std_ulogic;
  function or_one(d : std_ulogic_vector) return std_ulogic;
  function and_zero(d : std_ulogic_vector) return std_ulogic;
  function or_flip(d : std_ulogic_vector) return std_ulogic;
  function xor_first(d : std_ulogic_vector) return std_ulogic;
  function and_decl(a : std_logic_vector) return std_logic;
  function or_decl(a : std_logic_vector) return std_logic;
end package;
package body red_pkg is
  function or_f(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := v or d(i);
    end loop;
    return v;
  end function;
  function and_f(d : std_logic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '1';
    for i in d'range loop
      v := d(i) and v;
    end loop;
    return v;
  end function;
  function xor_f(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := v xor d(i);
    end loop;
    return v;
  end function;
  -- not folds: the start value is not the identity
  function or_one(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '1';
    for i in d'range loop
      v := v or d(i);
    end loop;
    return v;
  end function;
  function and_zero(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := v and d(i);
    end loop;
    return v;
  end function;
  -- not a fold: the loop runs backwards over the range (same result, still the body)
  function or_flip(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'reverse_range loop
      v := v or d(i);
    end loop;
    return v;
  end function;
  -- not a fold: a different gate than the start value suggests
  function xor_first(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := v xor d(i);
    end loop;
    return not v;
  end function;
  -- the start value as the variable's default
  function and_decl(a : std_logic_vector) return std_logic is
    variable z : std_logic := '1';
  begin
    for i in a'range loop
      z := z and a(i);
    end loop;
    return z;
  end function;
  function or_decl(a : std_logic_vector) return std_logic is
    variable z : std_logic := '0';
  begin
    for i in a'range loop
      z := a(i) or z;
    end loop;
    return z;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use work.red_pkg.all;
entity user_reduce_idiom is end;
architecture t of user_reduce_idiom is
  function or_local(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := v or d(i);
    end loop;
    return v;
  end function;
begin
  process
    constant all9 : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
    variable up   : std_ulogic_vector(0 to 2);
    variable down : std_ulogic_vector(2 downto 0);
    variable ra, rb, rx : std_ulogic;
    variable n : natural := 0;
    variable nul : std_ulogic_vector(1 to 0);
    procedure check(d : std_ulogic_vector) is
      variable o, a, x : std_ulogic;
    begin
      o := '0'; a := '1'; x := '0';
      for i in d'range loop
        o := o or d(i);
        a := a and d(i);
        x := x xor d(i);
      end loop;
      assert or_f(d) = o report "or_f " & to_string(d) severity failure;
      assert or_local(d) = o report "or_local " & to_string(d) severity failure;
      assert and_f(std_logic_vector(d)) = a report "and_f " & to_string(d) severity failure;
      assert xor_f(d) = x report "xor_f " & to_string(d) severity failure;
      assert and_decl(std_logic_vector(d)) = a report "and_decl " & to_string(d) severity failure;
      assert or_decl(std_logic_vector(d)) = o report "or_decl " & to_string(d) severity failure;
      assert or_flip(d) = o report "or_flip " & to_string(d) severity failure;
      assert xor_first(d) = not x report "xor_first " & to_string(d) severity failure;
      assert or_one(d) = '1' report "or_one " & to_string(d) severity failure;
      assert and_zero(d) = '0' report "and_zero " & to_string(d) severity failure;
      n := n + 1;
    end procedure;
  begin
    check(nul);
    assert or_f(nul) = '0' and and_f("") = '1' and xor_f(nul) = '0' severity failure;
    for i in 0 to 8 loop
      check(all9(i to i));
      for j in 0 to 8 loop
        check(all9(i to i) & all9(j to j));
        for k in 0 to 8 loop
          up := all9(i to i) & all9(j to j) & all9(k to k);
          down := up;
          check(up);
          check(down);
        end loop;
      end loop;
    end loop;
    report "vectors checked " & integer'image(n);
    wait;
  end process;
end architecture;
