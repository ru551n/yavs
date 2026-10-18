-- difftest: top=nested_alias_slices std=2008
-- Names of signals of arrays of arrays through a subtype alias with other bounds and below
-- a slice (#230): `al(i)(j)` with `al` indexed `1 to 4` over `m(0 to 3)`, `m(0 to 1)(1)`,
-- `m(1 to 3)(2 to 3)(3)`, as targets, in sensitivity lists, read, and for a signal of
-- records.
package nas_pkg is
  type w8 is array (natural range <>) of bit_vector(7 downto 0);
  subtype m4 is w8(0 to 3);
  subtype m4b is w8(1 to 4);
  type ia is array (1 to 3) of integer;
  type iaa is array (natural range <>) of ia;
  type rec is record a : bit_vector(3 downto 0); n : integer; end record;
  type recs is array (natural range <>) of rec;
end;

use work.nas_pkg.all;
entity nested_alias_slices is end;
architecture t of nested_alias_slices is
  signal m  : m4;
  signal q  : iaa(0 to 3) := (others => (others => 0));
  signal r  : recs(0 to 3) := (others => ("0000", 0));
  signal n0 : natural := 0;
  signal n1 : natural := 0;
  signal n2 : natural := 0;
  signal n3 : natural := 0;
  alias al : m4b is m;
  alias aq : iaa(1 to 4) is q;
begin
  -- Sensitive to one bit through the alias: m(1)(0).
  process (al(2)(0))
  begin
    n0 <= n0 + 1;
  end process;

  -- Sensitive to element 1 of the slice m(2 to 3): m(3).
  process (m(2 to 3)(3))
  begin
    n1 <= n1 + 1;
  end process;

  -- Sensitive to the slice of a slice m(0 to 2)(1 to 2): m(1) and m(2).
  process (m(0 to 2)(1 to 2))
  begin
    n2 <= n2 + 1;
  end process;

  -- Sensitive to an integer element through the alias: q(3)(2).
  process (aq(4)(2))
  begin
    n3 <= n3 + 1;
  end process;

  process
  begin
    al(2)(0) <= '1';
    m(0 to 1)(0) <= x"0f";
    aq(1)(1) <= 5;
    q(1 to 3)(2)(3) <= 7;
    r(1 to 2)(2).n <= 9;
    r(0 to 3)(1 to 2)(1).a <= "1010";
    wait for 1 ns;
    report "m=" & to_string(m(0)) & " " & to_string(m(1)) & " al(2)=" & to_string(al(2));
    report "q=" & integer'image(q(0)(1)) & integer'image(q(2)(3)) & integer'image(aq(3)(3))
      & " r=" & integer'image(r(2).n) & to_string(r(1).a);
    m(2 to 3)(3)(7 downto 4) <= "1001";
    m(0 to 2)(1 to 2) <= (x"aa", x"55");
    wait for 1 ns;
    report "m(1)=" & to_string(m(0 to 3)(1 to 2)(1)) & " m(2)=" & to_string(m(2))
      & " m(3)=" & to_string(m(3));
    aq(4)(2) <= 3;
    al(4)(7) <= '0';
    wait for 1 ns;
    report "n0=" & integer'image(n0) & " n1=" & integer'image(n1) & " n2="
      & integer'image(n2) & " n3=" & integer'image(n3) & " m(3)=" & to_string(al(4));
    assert q(3)(2) = 3 and m(1) = x"aa" severity failure;
    wait;
  end process;
end;
