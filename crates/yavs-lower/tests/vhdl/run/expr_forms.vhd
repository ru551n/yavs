-- difftest: top=expr_forms std=2008
library ieee;
use ieee.std_logic_1164.all;
entity expr_forms is end;
architecture t of expr_forms is
  type ivec is array (natural range <>) of integer;
  type rvec is array (natural range <>) of real;
  attribute tag : string;
  attribute width : integer;
  signal s : bit;
  attribute tag of s : signal is "sig";
  attribute width of s : signal is 12;
  procedure flip(x : inout std_ulogic) is
  begin
    x := not x;
  end;
  procedure two(a : out integer; b : out integer) is
  begin
    a := 1;
    b := 2;
  end;
begin
  process
    variable v : std_ulogic_vector(3 downto 0) := "0101";
    variable i : ivec(1 to 3) := (1, 2, 3);
    variable r : rvec(1 to 3);
    variable p, q : integer;
    variable bv : bit_vector(1 to 2) := "10";
    variable b0, b1 : bit;
  begin
    flip(v(2));
    flip(v(0));
    report "v=" & to_string(v);
    two(p, q);
    report "p=" & integer'image(p) & integer'image(q);
    r := rvec(i);
    report "r2=" & integer'image(integer(r(2) * 10.0));
    i := ivec(r);
    report "i3=" & integer'image(i(3));
    (p, q) := ivec'(7, 8);
    (b0, b1) := bv;
    report "pq=" & integer'image(p) & integer'image(q) & " b=" & bit'image(b0) & bit'image(b1);
    report s'tag & " " & integer'image(s'width + 1);
    wait;
  end process;
end;
