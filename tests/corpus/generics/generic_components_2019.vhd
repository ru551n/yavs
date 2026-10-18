-- difftest: top=generic_components_cfg std=2019
-- Components with generic types, subprograms and packages bound to entities with the same
-- generics: default binding, a configuration specification and a configuration
-- declaration, each a different map; ports of the generic type on both sides.
package ofs_pkg is
  generic (k : integer);
  function ofs(x : integer) return integer;
end;
package body ofs_pkg is
  function ofs(x : integer) return integer is begin return x + k; end;
end;

package ofs3 is new work.ofs_pkg generic map (k => 3);

entity cell is
  generic (type t; init : t; function f(x : t) return t);
  port (clk : in bit; q : out t);
end;
architecture once of cell is
  signal s : t := init;
begin
  q <= s;
  process (clk) begin
    if clk'event and clk = '1' then
      s <= f(s);
    end if;
  end process;
end;
architecture twice of cell is
  signal s : t := init;
begin
  q <= s;
  process (clk) begin
    if clk'event and clk = '1' then
      s <= f(f(s));
    end if;
  end process;
end;

entity shifter is
  generic (package p is new work.ofs_pkg generic map (<>); n : natural);
  port (y : out integer);
end;
architecture a of shifter is
  use p.all;
begin
  y <= ofs(n);
end;

entity generic_components is end;
architecture a of generic_components is
  component cell is
    generic (type t; init : t; function f(x : t) return t);
    port (clk : in bit; q : out t);
  end component;
  component shifter is
    generic (package p is new work.ofs_pkg generic map (<>); n : natural);
    port (y : out integer);
  end component;
  function inc(x : integer) return integer is begin return x + 1; end;
  function flip(x : bit) return bit is begin return not x; end;
  function rot(x : bit_vector) return bit_vector is
  begin
    return x(x'left + 1 to x'right) & x(x'left);
  end;
  signal clk : bit;
  signal qi, qs, qd, y : integer;
  signal qb : bit;
  signal qv : bit_vector(0 to 3);
  for us : cell use entity work.cell(twice);
begin
  -- Default binding (the last architecture, twice), two maps of the same component.
  ui : cell generic map (t => integer, init => 5, f => inc) port map (clk, qi);
  ub : cell generic map (t => bit, init => '0', f => flip) port map (clk, qb);
  -- Configuration specification.
  us : cell generic map (t => integer, init => 100, f => inc) port map (clk, qs);
  -- Configuration declaration (generic_components_cfg).
  ud : cell generic map (t => bit_vector(0 to 3), init => "1000", f => rot)
    port map (clk, qv);
  uk : shifter generic map (p => work.ofs3, n => 10) port map (y);
  process begin
    for i in 1 to 3 loop
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
      wait for 1 ns;
    end loop;
    report "qi=" & integer'image(qi) & " qb=" & bit'image(qb) & " qs=" & integer'image(qs)
      & " qv=" & to_string(qv) & " y=" & integer'image(y);
    assert qi = 11 and qb = '0' and qs = 106 and qv = "0100" and y = 13 severity failure;
    wait;
  end process;
end;

configuration generic_components_cfg of generic_components is
  for a
    for ud : cell use entity work.cell(once);
    end for;
  end for;
end;
