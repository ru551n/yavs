-- Qualified record aggregates `r_t'(...)` (`W` replaced by a width): assigned to a register, as an
-- operand of `=`, nested; one whose integer field may leave its subtype (`i + 1`) keeps its
-- process out, and with generic `fail` it does, failing as in the normal mode.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_qualrec_p is
  type in_t is record
    f : unsigned(3 downto 0);
  end record;
  type r_t is record
    a : integer range 0 to 3;
    b : std_ulogic_vector(W - 1 downto 0);
    c : boolean;
    d : std_ulogic;
    e : in_t;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_qualrec_p.all;
entity cycle_qualrec_u is
  port (clk : in std_ulogic; i : in integer range 0 to 3; x : in std_ulogic_vector(W - 1 downto 0);
        q, p : out r_t; eq : out std_ulogic);
end entity;
architecture a of cycle_qualrec_u is
  signal rr : r_t := (0, (others => '0'), false, '0', (f => "0000"));
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      rr <= r_t'(a => i, b => x, c => x(0) = '1', d => x(W - 1),
                 e => in_t'(f => to_unsigned(i, 4) + 1));
    end if;
  end process;
  q <= rr;
  eq <= '1' when r_t'(a => 1, b => x, c => false, d => x(W - 1), e => (f => "0010")) = rr
        else '0';
  over : process (clk)
  begin
    if rising_edge(clk) then
      p <= r_t'(a => i + 1, b => not x, c => true, d => '1', e => (f => "1111"));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_qualrec_p.all;
entity cycle_qualrec is
  generic (fail : boolean := false);
end entity;
architecture t of cycle_qualrec is
  signal clk : std_ulogic := '0';
  signal i : integer range 0 to 3 := 0;
  signal x : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal q, p : r_t;
  signal eq : std_ulogic;
begin
  u : entity work.cycle_qualrec_u port map (clk, i, x, q, p, eq);
  main : process
    variable s : unsigned(31 downto 0) := x"ACE1F00D";
  begin
    for k in 0 to 20 loop
      for b in x'range loop
        s := s(30 downto 0) & (s(31) xor s(21) xor s(1) xor s(0));
        x(b) <= s(0);
      end loop;
      i <= k mod 3;
      if fail and k = 7 then
        i <= 3;
      end if;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " " & integer'image(q.a) & " " & to_hstring(q.b) & " "
        & boolean'image(q.c) & " " & std_ulogic'image(q.d) & " " & to_hstring(q.e.f) & " "
        & integer'image(p.a) & " " & to_hstring(p.b) & " " & std_ulogic'image(eq);
    end loop;
    wait;
  end process;
end architecture;
