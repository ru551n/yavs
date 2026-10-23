-- difftest: top=integer_literal_outside exit=1 ref=hand
-- ref=hand: the reference simulators word it differently or accept it. A universal integer
-- literal converted to INTEGER lies in its range (LRM 9.3.6, 5.2.1): outside is an error when
-- evaluated, here as the argument of 'image (#463). yavs says "value V outside of INTEGER
-- range L to H".
entity integer_literal_outside is end;
architecture a of integer_literal_outside is
begin
  process
  begin
    report integer'image(2147483647);
    report integer'image(-2147483648);
    report integer'image(2147483648);
    report "not reached";
    wait;
  end process;
end;
