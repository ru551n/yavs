-- Byte codes: `e` and `r` have reads of one element and keep them; `w` and `b` are read whole or
-- by slice only (`b` is written by element, with repeated values), and `rc` is a record; the
-- report is the same with the codes of every signal (YAVS_CODES=all).
library ieee; use ieee.std_logic_1164.all;
entity elem_codes is end;
architecture a of elem_codes is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal e : std_logic_vector(15 downto 0) := x"A5C3";
  signal w : std_logic_vector(15 downto 0) := x"0001";
  signal b : std_logic_vector(99 downto 0) := (others => '0');
  signal r : std_logic_vector(99 downto 0) := (others => '0');
  type rec is record f : std_logic; g : std_logic_vector(7 downto 0); end record;
  signal rc : rec := ('0', x"01");
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      e <= e(14 downto 0) & (e(15) xor e(2));
      w <= w(14 downto 0) & w(15 downto 15);
      if n mod 3 = 0 then b(n mod 100) <= '1'; else b((n * 7) mod 100) <= '0'; end if;
      r((n * 3) mod 100) <= r((n + 1) mod 100) xor '1';
      rc.f <= not rc.f;
      rc.g <= rc.g(6 downto 0) & rc.f;
    end if;
  end process;
  process
  begin
    wait until n = 300;
    report "e=" & to_string(e) & " w=" & to_string(w) & " b=" & to_string(b) & " r=" & to_string(r)
      & " rc=" & std_logic'image(rc.f) & to_string(rc.g);
    done <= true; wait;
  end process;
end;
