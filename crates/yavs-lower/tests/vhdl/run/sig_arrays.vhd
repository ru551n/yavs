-- difftest: top=sig_arrays std=2008
library ieee;
use ieee.std_logic_1164.all;
entity cnt is
  port (d : in std_ulogic_vector; q : out std_ulogic_vector);
end;
architecture a of cnt is
begin
  process (d)
  begin
    q <= not d;
    report "cnt d=" & to_string(d) & " len=" & integer'image(d'length) & " left=" & integer'image(d'left);
  end process;
end;
library ieee;
use ieee.std_logic_1164.all;
entity sig_arrays is end;
architecture t of sig_arrays is
  signal a : std_ulogic_vector(7 downto 4) := "0011";
  signal b : std_ulogic_vector(3 downto 0);
  signal n : integer_vector(1 to 3) := (1, 2, 3);
  procedure show(signal s : in std_ulogic_vector; signal v : in integer_vector) is
  begin
    report "s=" & to_string(s) & " s(5)=" & std_ulogic'image(s(5)) & " v(2)=" & integer'image(v(2))
      & " sl=" & to_string(s(6 downto 5));
  end;
  procedure set(signal s : out std_ulogic_vector; signal v : inout integer_vector) is
  begin
    s <= "1010";
    v(3) <= v(1) + 40;
  end;
begin
  u : entity work.cnt port map (d => a, q => b);
  process
  begin
    wait for 1 ns;
    show(a, n);
    set(a, n);
    wait for 1 ns;
    show(a, n);
    report "b=" & to_string(b);
    wait;
  end process;
end;
