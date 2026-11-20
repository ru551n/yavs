-- LRM 14.4.2.1: the range of a variable's anonymous subtype is evaluated when the declaration
-- is elaborated; changing the variable it reads later does not change it (#1174).
entity top is end entity;
architecture a of top is
begin
  process
    variable k : integer := 9;
    variable q : integer range 0 to k := 0;
  begin
    k := 1;
    q := 5;
    report "RES q=" & integer'image(q);
    wait;
  end process;
end architecture;
