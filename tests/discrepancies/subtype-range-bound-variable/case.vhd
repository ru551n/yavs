-- LRM 14.4.2.1: the range of a scalar subtype declared in a subprogram is evaluated when the
-- declaration is elaborated; changing the variable it reads later does not change it (#612).
entity top is end entity;
architecture a of top is
  procedure p is
    variable n : integer := 3;
    subtype sr is integer range 0 to n;
    variable z : sr;
  begin
    n := 7;
    z := sr'high;
    report "RES sr'high=" & integer'image(sr'high) & " z=" & integer'image(z);
  end procedure;
begin
  process
  begin
    p;
    wait;
  end process;
end architecture;
