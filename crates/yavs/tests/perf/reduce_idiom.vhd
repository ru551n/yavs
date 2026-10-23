-- A user function that folds a std_ulogic vector with and/or/xor (the reduction idiom) called
-- 100 times: lowered to one word-parallel native, not a loop of seven operations per bit.
library ieee; use ieee.std_logic_1164.all;
entity reduce_idiom is end;
architecture a of reduce_idiom is
  function all_ones(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic := '1';
  begin
    for i in d'range loop v := v and d(i); end loop;
    return v;
  end function;
  function any_one(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic := '0';
  begin
    for i in d'range loop v := v or d(i); end loop;
    return v;
  end function;
begin
  process
    variable w : std_ulogic_vector(31 downto 0) := x"0F0F_F0F1";
    variable n1, n2 : natural := 0;
  begin
    for k in 1 to 100 loop
      w := w(30 downto 0) & w(31);
      if all_ones(w(7 downto 0)) = '1' then n1 := n1 + 1; end if;
      if any_one(w) = '1' then n2 := n2 + 1; end if;
    end loop;
    report "n1=" & integer'image(n1) & " n2=" & integer'image(n2);
    wait;
  end process;
end;
