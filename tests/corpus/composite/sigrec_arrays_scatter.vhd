-- difftest: top=sigrec_arrays_scatter std=2008 stop=30ns
-- Parts of record signals that are driven once in their process (#677), so that one instruction
-- drives each: an element by a run-time index, a slice, a field that is an array of records, a field
-- that is a record, an element of such a field, with and without a delay, one process for each
-- signal. (`sigrec_arrays` drives the same signals again and again, which keeps the leaf-by-leaf
-- code the optimiser can shorten.)
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
entity sigrec_arrays_scatter is end;
architecture a of sigrec_arrays_scatter is
  signal se, sf, sl : cells_to;
  signal sd : cells_dt;
  signal g, h : grp;
  signal gs : grps;
  signal n : natural := 0;
begin
  p_elem : process
  begin
    wait for 1 ns;
    for i in se'range loop
      se(i) <= mk(i + 10);
    end loop;
    wait;
  end process;

  p_slice : process
  begin
    wait for 2 ns;
    sf(1 to 2) <= (mk(20), mk(21));
    wait;
  end process;

  p_desc : process
  begin
    wait for 1 ns;
    for i in sd'range loop
      sd(i) <= mk(30 + i) after 2 ns;
    end loop;
    wait;
  end process;

  p_field : process
  begin
    wait for 3 ns;
    g.cs <= (mk(40), mk(41), mk(42), mk(43));
    g.inner <= mk(44);
    wait;
  end process;

  p_inner : process
  begin
    wait for 4 ns;
    h.inner <= mk(50) after 1 ns;
    h.k <= 7;
    wait;
  end process;

  p_gs : process
  begin
    wait for 5 ns;
    for j in 0 to 3 loop
      gs(2).cs(j) <= mk(60 + j);
    end loop;
    wait;
  end process;

  p_sl : process
    variable va : cells_to;
  begin
    wait for 6 ns;
    for i in va'range loop
      va(i) := mk(70 + i);
    end loop;
    sl <= va;
    n <= 1;
    wait;
  end process;

  chk : process
    variable t : natural := 0;
  begin
    for step in 1 to 10 loop
      wait for 1.5 ns;
      t := t + 1;
      report integer'image(t) & " se=" & img(se);
      report integer'image(t) & " sf=" & img(sf);
      report integer'image(t) & " sd=" & img(sd);
      report integer'image(t) & " g=" & img(g);
      report integer'image(t) & " h=" & img(h);
      report integer'image(t) & " gs2=" & img(gs(2)) & " gs1=" & img(gs(1).cs(0));
      report integer'image(t) & " sl=" & img(sl) & integer'image(n);
    end loop;
    wait;
  end process;
end;
