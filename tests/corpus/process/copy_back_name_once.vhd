-- difftest: top=copy_back_name_once std=2008
-- The actual of an out or inout parameter is the object its name denotes when the call is
-- made: an index the procedure changes, or an impure function in the index, does not move
-- the write-back of a value passed through a temporary (an element of an integer array, of
-- an array of a time subtype, a logic element and a logic slice; LRM 4.2.2.2, #1182).
library ieee;
use ieee.std_logic_1164.all;
entity copy_back_name_once is end entity;
architecture a of copy_back_name_once is
  subtype short is time range 0 ns to 10 ns;
  type iarr is array (0 to 3) of integer;
  type tarr is array (0 to 3) of short;
  procedure nxti (variable o : out integer; variable i : inout integer) is
  begin
    o := 3;
    i := i + 1;
  end procedure;
  procedure nxtt (variable o : out time; variable i : inout integer) is
  begin
    o := 3 ns;
    i := i + 1;
  end procedure;
  procedure nxtl (variable o : out std_ulogic; variable i : inout integer) is
  begin
    o := '1';
    i := i + 1;
  end procedure;
  procedure nxtv (variable o : out std_ulogic_vector; variable i : inout integer) is
  begin
    o := (o'range => '1');
    i := i + 2;
  end procedure;
  procedure seti (variable o : out integer; v : integer) is begin o := v; end procedure;
begin
  process
    variable b : iarr := (others => 0);
    variable t : tarr := (others => 0 ns);
    variable v : std_ulogic_vector(0 to 7) := (others => '0');
    variable i : integer;
    variable cnt : integer := 0;
    impure function bump return integer is
    begin
      cnt := cnt + 1;
      return 2;
    end function;
  begin
    i := 1;
    nxti(b(i), i);
    report "b(1)=" & integer'image(b(1)) & " b(2)=" & integer'image(b(2));
    i := 1;
    nxtt(t(i), i);
    report "t(1)=" & time'image(t(1)) & " t(2)=" & time'image(t(2));
    i := 1;
    nxtl(v(i), i);
    i := 4;
    nxtv(v(i to i + 1), i);
    report "v=" & to_string(v);
    seti(b(bump), 4);
    report "b(2)=" & integer'image(b(2)) & " cnt=" & integer'image(cnt);
    assert b(1) = 3 and t(1) = 3 ns and v = "01001100" and b(2) = 4 and cnt = 1
      severity failure;
    wait;
  end process;
end architecture;
