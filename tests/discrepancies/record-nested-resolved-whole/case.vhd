-- (from the differential corpus)
-- ref=hand: two of the reference simulators give these values; the first resolves the
-- elements with each other's values, the second rejects a process driving part of one.
-- Records resolved as a whole as elements of an unresolved record (LRM 4.6, 14.7.3.1): each
-- such element is a basic signal with its own sources; a process driving one field of it
-- drives all of its fields (LRM 14.7.2), and the resolution of a resolved field inside it
-- (std_logic) is not used.
library ieee;
use ieee.std_logic_1164.all;
entity nested_record_whole is end;
architecture t of nested_record_whole is
  type rec is record l : std_logic; n : integer; c : character; end record;
  type rec_vec is array (natural range <>) of rec;
  -- Order independent: l from the source count, n the sum, c the largest.
  function res(v : rec_vec) return rec is
    variable r : rec := ('0', 0, 'a');
  begin
    if v'length > 1 then r.l := 'H'; end if;
    for i in v'range loop
      r.n := r.n + v(i).n;
      if v(i).c > r.c then r.c := v(i).c; end if;
    end loop;
    return r;
  end;
  subtype rrec is res rec;
  type outer is record a : rrec; k : integer; b : rrec; end record;
  signal s : outer := (a => ('0', 1, 'b'), k => 7, b => ('1', 2, 'c'));
  function img(r : rec) return string is
  begin
    return std_logic'image(r.l) & " " & integer'image(r.n) & " " & character'image(r.c);
  end;
begin
  -- s.a: p1 and p2 (p2 only its n); s.b: p3 alone; s.k: p1.
  p1 : process begin
    s.a <= ('1', 10, 'x') after 1 ns;
    s.k <= 8 after 1 ns;
    wait;
  end process;
  p2 : process begin
    wait for 2 ns;
    s.a.n <= 5;
    wait;
  end process;
  p3 : process begin
    s.b.c <= 'q' after 1 ns;
    wait for 3 ns;
    s.b <= ('Z', 40, 'd');
    wait;
  end process;
  process (s.a) begin report "a " & img(s.a); end process;
  process (s.b) begin report "b " & img(s.b); end process;
  process (s.k) begin report "k " & integer'image(s.k); end process;
  process begin
    wait for 5 ns;
    assert s.a = ('H', 15, 'x') and s.b = ('0', 40, 'd') and s.k = 8 severity failure;
    report "done";
    wait;
  end process;
end;
