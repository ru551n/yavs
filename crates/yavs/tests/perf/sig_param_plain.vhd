-- Signal formals whose actual holds every value of the formal's subtype (the same subtype, a
-- `natural` formal on an `integer` signal, a `std_ulogic` formal on a `std_logic` signal): the
-- calls take the plain body, with no bound cells and no check per drive (#1181).
library ieee;
use ieee.std_logic_1164.all;
entity sig_param_plain is end;
architecture t of sig_param_plain is
  subtype digit is integer range 0 to 9;
  signal a : digit := 0;
  signal b : integer := 0;
  signal c : std_logic := '0';
  procedure pd (signal o : out digit; v : digit) is begin o <= v; end procedure;
  procedure pn (signal o : out natural; v : natural) is begin o <= v; end procedure;
  procedure pl (signal o : out std_ulogic; v : std_ulogic) is begin o <= v; end procedure;
begin
  process
    variable sum : natural := 0;
  begin
    for i in 1 to 50 loop
      pd(a, i mod 10);
      pn(b, i);
      pl(c, '1');
      wait for 1 ns;
      sum := sum + a + b;
    end loop;
    report "sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
