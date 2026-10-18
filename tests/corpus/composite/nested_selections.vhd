-- difftest: top=nested_selections std=2008
-- Nested selections of signals of arrays of arrays (#89): element and slice targets below
-- an element (`m(i)(j)`, `m(i)(7 downto 4)`), integer arrays of arrays, two-dimensional
-- indices, static and computed indices, nested sensitivity, signal parameters, aliases and
-- port actuals.
entity ns_count is
  port (
    i : in  bit;
    o : out natural
  );
end;
architecture t of ns_count is
begin
  process (i)
    variable c : natural := 0;
  begin
    c := c + 1;
    o <= c;
  end process;
end;

package ns_pkg is
  type w8 is array (natural range <>) of bit_vector(7 downto 0);
  subtype m4 is w8(0 to 3);
  type ia is array (1 to 3) of integer;
  type iaa is array (0 to 1) of ia;
  type grid is array (0 to 2, 1 to 2) of integer;
  procedure set_bit(signal m : inout w8; w, b : natural);
end;

package body ns_pkg is
  procedure set_bit(signal m : inout w8; w, b : natural) is
  begin
    m(w)(b) <= '1';
    m(w + 1)(b + 1 downto b) <= "11";
  end;
end;

use work.ns_pkg.all;
entity nested_selections is end;
architecture t of nested_selections is
  signal m   : m4;
  signal p   : m4;
  signal q   : iaa := (others => (others => 0));
  signal g   : grid := (others => (others => 0));
  signal n0  : natural := 0;
  signal n1  : natural := 0;
  signal nm  : natural;
  alias top_word : bit_vector(7 downto 0) is m(3);
begin
  -- A nested port actual.
  cnt : entity work.ns_count port map (i => m(2)(0), o => nm);

  -- Sensitive to one bit of word 0 only.
  process (m(0)(0))
  begin
    n0 <= n0 + 1;
  end process;

  -- Sensitive to a slice of word 1.
  process (m(1)(7 downto 4))
  begin
    n1 <= n1 + 1;
  end process;

  process
    variable k : natural := 1;
  begin
    m(1)(3) <= '1';
    m(2)(7 downto 4) <= "1010";
    q(1)(2) <= 42;
    q(0)(1 to 2) <= (7, 8);
    g(2, 1) <= 5;
    wait for 1 ns;
    report "m(1)=" & to_string(m(1)) & " m(2)=" & to_string(m(2));
    report "q=" & integer'image(q(0)(1)) & integer'image(q(0)(2)) & integer'image(q(0)(3))
      & " " & integer'image(q(1)(1)) & integer'image(q(1)(2)) & integer'image(q(1)(3));
    report "g(2,1)=" & integer'image(g(2, 1)) & " g(1,2)=" & integer'image(g(1, 2));
    assert m(1)(3) = '1' and m(1) = "00001000" severity failure;
    assert m(2)(7 downto 4) = "1010" severity failure;
    assert q(1)(2) = 42 and q(0)(2) = 8 severity failure;
    -- Computed indices.
    for i in 0 to 3 loop
      m(i)(i) <= '1';
    end loop;
    q(k)(k + 2) <= 9;
    g(k, k + 1) <= 6;
    wait for 1 ns;
    report "m(3)=" & to_string(m(3)) & " m(0)(0)=" & bit'image(m(0)(0));
    assert q(1)(3) = 9 and g(1, 2) = 6 severity failure;
    m(1)(5) <= '1';
    wait for 1 ns;
    m(1)(5) <= '0';
    m(1)(2) <= '1';
    wait for 1 ns;
    -- Through an alias and a signal parameter.
    top_word(6) <= '1';
    set_bit(p, 1, 4);
    wait for 1 ns;
    report "m(3)=" & to_string(m(3)) & " p(1)=" & to_string(p(1)) & " p(2)=" & to_string(p(2));
    assert top_word = "01001000" severity failure;
    m(2)(0) <= '1';
    wait for 1 ns;
    report "n0=" & integer'image(n0) & " n1=" & integer'image(n1) & " nm=" & integer'image(nm);
    -- Whole elements and slices of words.
    p(0) <= x"f0";
    p(2 to 3) <= (x"11", x"22");
    q(1) <= (4, 5, 6);
    wait until p(3)(1) = '1';
    report "p=" & to_string(p(0)) & " " & to_string(p(1)) & " " & to_string(p(2)) & " "
      & to_string(p(3)) & " q(1)(3)=" & integer'image(q(1)(3));
    wait;
  end process;
end;
