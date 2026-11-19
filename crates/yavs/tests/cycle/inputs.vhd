-- Cycle mode: the region reads only the inputs that changed since its last wake. Inputs read
-- only by the clocked member (no wake) change between edges, change and change back, change in
-- the delta of the edge and at time 0; a wide input changes in its middle word only; an integer;
-- a metavalue input changes once (its metavalues counted once); of a record input, kept in one
-- word, only the field at element 0 changes; records whose word holds an input field and a field
-- a member drives, the input first (`p`) and second (`p2`). `fb`, outside, reads a region output
-- and drives an input back.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_inputs is
end entity;
architecture a of cycle_inputs is
  type pair_t is record
    a, b : std_ulogic;
  end record;
  signal r : pair_t := ('0', '1');
  signal p, p2 : pair_t := ('0', '0');
  signal qr : std_ulogic_vector(1 downto 0) := "00";
  signal clk, e, z, f : std_ulogic := '0';
  signal s, qs : std_ulogic_vector(7 downto 0) := x"00";
  signal w, qw : std_ulogic_vector(199 downto 0) := (others => '0');
  signal n, qn : integer := 0;
  signal c, y : std_ulogic_vector(3 downto 0) := "0000";
  signal m : std_ulogic_vector(3 downto 0) := "UUUU";
  signal qm : std_ulogic_vector(3 downto 0) := "0000";
  signal wm : std_ulogic_vector(30 downto 0) := (others => '0');
  signal qe, qz, qf : std_ulogic := '0';
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      qs <= s;
      qw <= w;
      qn <= n;
      qe <= e;
      qz <= z;
      qf <= f;
      qm <= m;
      qr <= r.a & r.b;
      -- `p.a` and `p2.b` come from `main`.
      p.b <= not p.b xor p.a;
      p2.a <= not p2.a xor p2.b;
    end if;
  end process;
  y <= c xor qs(3 downto 0);
  wm <= w(130 downto 100);
  fb : process (y)
  begin
    f <= y(1) after 1 ns;
  end process;
  main : process
  begin
    z <= '1';
    for i in 1 to 8 loop
      -- `e` changes in the delta of the edge.
      clk <= '1';
      e <= not e;
      wait for 1 ns;
      -- Changed and changed back between edges, the region not woken (`i` odd), or changed.
      s <= x"a5";
      wait for 1 ns;
      if i mod 2 = 1 then
        s <= x"00";
      end if;
      -- A glitch over delta cycles.
      w(70) <= '1';
      wait for 0 ns;
      w(70) <= '0';
      w(100 + i) <= '1';
      n <= 7 * i;
      r.a <= not r.a;
      p.a <= not p.a;
      p2.b <= not p2.b;
      if i = 3 then
        m <= "XX01";
      end if;
      c <= std_ulogic_vector(to_unsigned(i, 4));
      wait for 3 ns;
      clk <= '0';
      wait for 5 ns;
      report "i=" & integer'image(i) & " qs=" & to_hstring(qs) & " qw=" & to_hstring(qw(127 downto 64))
        & " qn=" & integer'image(qn) & " qe=" & to_string(qe) & " qz=" & to_string(qz)
        & " qf=" & to_string(qf) & " qm=" & to_string(qm) & " y=" & to_string(y)
        & " wm=" & to_hstring(wm) & " qr=" & to_string(qr)
        & " pb=" & to_string(p.b) & " p2a=" & to_string(p2.a);
    end loop;
    wait;
  end process;
end architecture;
