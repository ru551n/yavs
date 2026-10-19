-- LRM 6.5.7.1: an actual conversion of an in variable parameter.
entity top is end entity;
architecture a of top is
  procedure geti (variable x : in integer) is begin report "RES " & integer'image(x); end procedure;
begin
  process
    variable r : real := 2.0;
  begin
    geti(integer(r));
    wait;
  end process;
end architecture;
