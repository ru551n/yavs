-- difftest: top=const_elem_constraint std=2008
-- Elements of a constant of an array subtype with an element constraint
-- (`ab(1 to 3)(7 downto 0)` of `array (natural range <>) of std_ulogic_vector`) take that
-- constraint's index range (LRM 5.3.2.2, 6.4.2.2), not the literal's `0 to n - 1`: bits and
-- slices read during elaboration (signal initial values, constants, port actuals) and at run
-- time, both directions, ranges not from 0, in a package and an architecture, and a record
-- field. Analysis kept the literal's range when the subtype had an element constraint of its
-- own: a reversed bit or "slice direction differs from its prefix". Found by the cycle mode
-- fuzzer.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package const_elem_constraint_pkg is
  type ub is array (natural range <>) of unsigned;
  subtype ud is ub(4 downto 2)(12 downto 5);
  constant PD : ud := (x"A5", "00011110", x"C3");
  subtype ua is ub(0 to 1)(3 to 10);
  constant PA : ua := (others => x"96");
  type rb is record
    a : std_ulogic_vector;
    n : natural;
  end record;
  subtype rc is rb(a(9 downto 2));
  constant PR : rc := (a => x"56", n => 3);
end package;

library ieee;
use ieee.std_logic_1164.all;
entity const_elem_constraint_sub is
  port (a : in std_ulogic_vector(2 downto 0); b : in std_ulogic);
end entity;
architecture a of const_elem_constraint_sub is
begin
  process
  begin
    report "port a=" & to_hstring(a) & " b=" & std_ulogic'image(b);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.const_elem_constraint_pkg.all;
entity const_elem_constraint is end entity;
architecture a of const_elem_constraint is
  type ab is array (natural range <>) of std_ulogic_vector;
  subtype at is ab(1 to 3)(7 downto 0);
  constant K : at := (x"12", "00110100", x"56");
  constant S : std_ulogic_vector(2 downto 0) := K(2)(5 downto 3);
  constant C1 : std_ulogic := K(3)(2);
  constant C2 : unsigned(3 downto 0) := PD(4)(12 downto 9);
  signal s1 : std_ulogic := K(3)(2);
  signal s2 : std_ulogic_vector(2 downto 0) := K(3)(4 downto 2);
  signal s3 : std_ulogic := PD(3)(6);
  signal s4 : unsigned(3 downto 0) := PA(1)(3 to 6);
  signal s5 : std_ulogic := PA(0)(10);
  signal s6 : std_ulogic_vector(2 downto 0) := PR.a(6 downto 4);
begin
  u : entity work.const_elem_constraint_sub port map (a => K(1)(6 downto 4), b => PD(2)(5));
  process
    variable i : integer := 2;
  begin
    report "S=" & to_hstring(S) & " C1=" & std_ulogic'image(C1) & " C2=" & to_hstring(C2) & " s1=" &
      std_ulogic'image(s1) & " s2=" & to_hstring(s2) & " s3=" & std_ulogic'image(s3) & " s4=" &
      to_hstring(s4) & " s5=" & std_ulogic'image(s5) & " s6=" & to_hstring(s6) & " rt=" &
      to_hstring(K(i)(5 downto 3)) & std_ulogic'image(K(i + 1)(2)) & to_hstring(PD(i + 1)(8 downto 5)) & integer'image(K(1)'left);
    wait;
  end process;
end architecture;
