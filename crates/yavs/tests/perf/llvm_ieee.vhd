-- numeric_std operations on small vectors in compiled code: computed in place (no callback)
-- in a function (native frame) and in a process (frame mode); the one operation that warns
-- (a metavalue) and the one that needs more than a word (65 elements) take the callback.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity llvm_ieee is end;
architecture a of llvm_ieee is
  function mix(x, y : unsigned; k : natural) return integer is
    variable s : signed(x'length - 1 downto 0) := signed(x);
    variable n : integer := 0;
  begin
    if x < y then n := n + 1; end if;
    if s >= to_signed(k - 20, x'length) then n := n + 2; end if;
    if x = k then n := n + 4; end if;
    return n + to_integer(resize(x, 12) + (y srl (k mod 5))) + to_integer((x and y) xor shift_left(x, k mod 4));
  end function;
begin
  process
    variable a : unsigned(7 downto 0) := x"5A";
    variable b : unsigned(7 downto 0) := x"C3";
    variable m : unsigned(3 downto 0) := "1X10";
    variable w : unsigned(64 downto 0) := (others => '1');
    variable acc : integer := 0;
  begin
    for k in 1 to 100 loop
      a := a + 1;
      b := b + a;
      acc := (acc + mix(a, b, k) + to_integer(to_unsigned(k mod 64, 6) rol 2)) mod 100000;
    end loop;
    acc := acc + to_integer(m);
    w := w + 1;
    acc := acc + to_integer(w(3 downto 0));
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
