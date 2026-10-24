-- difftest: top=sigrec_arrays std=2008 stop=30ns
-- Arrays of records as signals (#677): whole, element (constant and run-time index), slice and
-- field parts of record signals whose fields are records, arrays of records or arrays of scalars,
-- read into variables and driven from variables: ascending and descending arrays, a delayed
-- drive, records that hold arrays of records, an array of those, and a port collapsed onto a
-- slice of its actual.
library ieee; use ieee.std_logic_1164.all;
package sigrec_arrays_pkg is
  type color is (red, green, blue);
  type ints is array (0 to 1) of integer range -100 to 100;
  type cell is record
    p : std_logic;
    v : std_logic_vector(3 downto 0);
    n : integer;
    c : color;
    ok : boolean;
    iv : ints;
  end record;
  type cells is array (natural range <>) of cell;
  subtype cells_to is cells(0 to 3);
  subtype cells_dt is cells(5 downto 3);
  subtype cells_8 is cells(0 to 7);
  type grp is record
    tag : std_logic;
    cs : cells_to;
    inner : cell;
    k : integer;
  end record;
  type grps is array (1 to 2) of grp;
  function mk (i : integer) return cell;
  function img (c : cell) return string;
  function img (a : cells) return string;
  function img (g : grp) return string;
end package;
package body sigrec_arrays_pkg is
  function mk (i : integer) return cell is
    variable r : cell;
  begin
    r.p := '1' when i mod 2 = 1 else '0';
    for b in 0 to 3 loop
      r.v(b) := '1' when ((i / (2 ** b)) mod 2) = 1 else '0';
    end loop;
    r.n := i * 7 - 3;
    r.c := color'val(i mod 3);
    r.ok := i mod 3 = 0;
    r.iv := (i, -i);
    return r;
  end;
  function img (c : cell) return string is
  begin
    return std_logic'image(c.p) & "/" & to_string(c.v) & "/" & integer'image(c.n) & "/" &
      color'image(c.c) & "/" & boolean'image(c.ok) & "/" & integer'image(c.iv(0)) & ":" &
      integer'image(c.iv(1));
  end;
  function img (a : cells) return string is
    variable s : string(1 to 400);
    variable n : natural := 0;
  begin
    for i in a'range loop
      s(n + 1 to n + img(a(i))'length + 1) := img(a(i)) & "|";
      n := n + img(a(i))'length + 1;
    end loop;
    return s(1 to n);
  end;
  function img (g : grp) return string is
  begin
    return std_logic'image(g.tag) & " " & img(g.cs) & " " & img(g.inner) & " " & integer'image(g.k);
  end;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.sigrec_arrays_pkg.all;
entity sigrec_arrays_unit is
  port (clk : in std_logic; d : in cells; q : out cells);
end;
architecture a of sigrec_arrays_unit is
begin
  process (clk)
    variable v : cells(d'range);
  begin
    if rising_edge(clk) then
      v := d;
      for i in v'range loop
        v(i).n := v(i).n + 1000;
      end loop;
      q <= v;
    end if;
  end process;
end;

library ieee; use ieee.std_logic_1164.all;
use work.sigrec_arrays_pkg.all;
entity sigrec_arrays is end;
architecture a of sigrec_arrays is
  signal clk : std_logic := '0';
  signal sa : cells_to;
  signal sd : cells_dt;
  signal g : grp;
  signal gs : grps;
  signal big, bigq : cells_8;
  signal sl : cells_to;
begin
  clk <= not clk after 5 ns;

  u : entity work.sigrec_arrays_unit port map (clk, big(2 to 5), bigq(0 to 3));

  ctl : process
    variable va : cells_to;
    variable vg : grp;
  begin
    -- Whole drive of an array of records from an aggregate of function results.
    sa <= (mk(0), mk(1), mk(2), mk(3));
    sd <= (mk(5), mk(4), mk(3));
    wait for 1 ns;
    -- One element at a time, by a run-time index, then the whole array from a variable.
    for i in sa'range loop
      sa(i) <= mk(i + 10);
    end loop;
    for i in sd'range loop
      sd(i) <= mk(i + 20);
    end loop;
    wait for 1 ns;
    va := sa;
    va(1).n := 77;
    va(2) := mk(33);
    sl <= va;
    -- A slice, constant and run-time bounds.
    sa(1 to 2) <= (mk(40), mk(41));
    for i in 0 to 1 loop
      sd(5 - i downto 4 - i) <= (mk(50 + i), mk(60 + i));
    end loop;
    wait for 1 ns;
    -- Fields that are arrays of records and records.
    vg := (tag => '1', cs => sa, inner => mk(70), k => 5);
    g <= vg;
    gs(1) <= vg;
    wait for 1 ns;
    g.cs <= sl;
    g.inner <= mk(71);
    gs(2).cs(2) <= mk(72);
    gs(2).inner <= mk(73);
    gs(2).cs(1).iv <= (8, 9);
    for i in 0 to 3 loop
      g.cs(i).n <= 100 * i;
    end loop;
    wait for 1 ns;
    -- Delayed drives of a part.
    sa(0) <= mk(80) after 5 ns;
    sd(4) <= mk(81) after 3 ns;
    g.cs(3) <= mk(82) after 4 ns;
    gs(1).cs <= va after 2 ns;
    -- The inputs of the port, from the same variable.
    big(2 to 5) <= va;
    big(0) <= mk(90);
    big(7) <= mk(91);
    wait;
  end process;

  chk : process
    variable va : cells_to;
    variable vd : cells_dt;
    variable vg : grp;
    variable two : cells(0 to 1);
    variable t : natural := 0;
  begin
    for step in 1 to 12 loop
      wait for 1.5 ns;
      t := t + 1;
      va := sa;
      vd := sd;
      vg := g;
      report integer'image(t) & " sa=" & img(va);
      report integer'image(t) & " sd=" & img(vd);
      report integer'image(t) & " g=" & img(vg);
      report integer'image(t) & " sl=" & img(sl);
      two := sa(1 to 2);
      report integer'image(t) & " two=" & img(two(0)) & " " & img(two(1));
      report integer'image(t) & " ge=" & img(sa(2)) & " " & img(sd(4)) & " " & img(g.inner);
      report integer'image(t) & " gs1=" & img(gs(1)) & " gs2=" & img(gs(2));
      report integer'image(t) & " gcs=" & img(g.cs) & " c2=" & img(g.cs(2));
      report integer'image(t) & " bigq=" & img(bigq(0 to 3)) & " big=" & img(big(2 to 5));
    end loop;
    wait;
  end process;
end;
