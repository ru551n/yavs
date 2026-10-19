-- difftest: top=concat_composite std=2008
-- `&` on arrays of composites (#234): arrays of words, of integer arrays and of records,
-- array & array, array & element, element & array and element & element, as variables,
-- signals, package and architecture constants and generic actuals; bounds from the index
-- subtype (LRM 9.2.5), also a descending one; an element subtype the type leaves open takes
-- the left operand's.
package concat_composite_pkg is
  type wv is array (natural range <>) of bit_vector(3 downto 0);
  type ia is array (1 to 2) of integer;
  type iav is array (natural range <>) of ia;
  type pt is record x : integer; en : boolean; end record;
  type pts is array (natural range <>) of pt;
  subtype didx is integer range 9 downto 0;
  type dwv is array (didx range <>) of bit_vector(1 downto 0);
  constant ws : wv := ("0001", "0110");
  constant wx : wv := ws & "0101";
  constant wy : wv := "1000" & ws;
  constant wz : wv := ws & wx;
  constant we : wv := bit_vector'("1111") & bit_vector'("0000");
  constant pq : pts := pts'((1, true), (2, false)) & pt'(3, true);
  function img(v : wv) return string;
  function img(v : iav) return string;
  function img(v : pts) return string;
  function dwi(v : dwv) return string;
end package;

package body concat_composite_pkg is
  function img(v : wv) return string is
  begin
    if v'length = 0 then return ""; end if;
    return to_string(v(v'left)) & "," & img(v(v'left + 1 to v'right));
  end;
  function img(v : iav) return string is
  begin
    if v'length = 0 then return ""; end if;
    return integer'image(v(v'left)(1)) & ":" & integer'image(v(v'left)(2)) & ","
      & img(v(v'left + 1 to v'right));
  end;
  function img(v : pts) return string is
  begin
    if v'length = 0 then return ""; end if;
    return integer'image(v(v'left).x) & ":" & boolean'image(v(v'left).en) & ","
      & img(v(v'left + 1 to v'right));
  end;
  function dwi(v : dwv) return string is
  begin
    return integer'image(v'left) & " downto " & integer'image(v'right) & " "
      & to_string(v(v'left)) & to_string(v(v'right));
  end;
end package body;

use work.concat_composite_pkg.all;
entity concat_composite_child is
  generic (g : wv; h : pts);
end;
architecture t of concat_composite_child is
begin
  process
  begin
    report "generic " & img(g) & " " & integer'image(g'left) & " to "
      & integer'image(g'right) & " " & img(h);
    wait;
  end process;
end;

use work.concat_composite_pkg.all;
entity concat_composite is end;
architecture t of concat_composite is
  constant ca : iav := ia'(1, 2) & ia'(3, 4);
  constant cb : iav := ca & ca & ia'(5, 6);
  type ow is array (natural range <>) of bit_vector;
  function oi(v : ow) return string is
  begin
    if v'length = 0 then return ""; end if;
    return to_string(v(v'left)) & "," & oi(v(v'left + 1 to v'right));
  end;
  constant co : ow := ow'(x"1", x"2") & x"3";
  signal sw : wv(0 to 3);
  signal si : iav(0 to 2);
  signal sp : pts(0 to 2);
begin
  u : entity work.concat_composite_child
    generic map (g => ws & "1010", h => pt'(9, false) & pq);
  process
    variable a : wv(0 to 1) := ("0011", "1100");
    variable b : wv(0 to 4);
    variable e : wv(0 to -1);
    variable x : iav(0 to 1) := ((7, 8), (9, 10));
    variable y : iav(0 to 2);
    variable p : pts(0 to 1) := ((4, false), (5, true));
    variable q : pts(0 to 3);
    variable d : dwv(9 downto 8) := ("01", "10");
    variable r : wv(0 to 1);
    variable oa : ow(0 to 1)(3 downto 0) := (x"a", x"b");
    variable oe : ow(0 to -1)(3 downto 0);
    variable ob : ow(0 to 2)(1 to 4);
  begin
    report "pkg " & img(wx) & " " & img(wy) & " " & img(wz) & " " & img(we)
      & " " & integer'image(wz'right) & " " & img(pq);
    report "arch " & img(ca) & " " & img(cb) & " " & integer'image(cb'length);
    b := a & ws & "1001";
    report "var " & img(b) & " " & img(e & a) & " " & img(a & e);
    y := x & ia'(11, 12);
    report img(y) & " " & img(ia'(0, 1) & x);
    q := p & p;
    report img(q) & " " & img(p & pt'(6, true)) & " " & img(pt'(6, true) & pt'(7, false));
    assert dwv'(d & "11") = dwv'("01", "10", "11") severity failure;
    report dwi(d & "11") & " " & dwi("00" & d) & " " & dwi("00" & "11");
    r := e & "0111" & e & "1110";
    report img(r);
    report "open " & oi(co) & " " & oi(oa & x"c") & " " & oi(x"d" & oa) & " "
      & oi(x"e" & x"f") & " " & oi(oe & oa);
    ob := oa(1 to 1) & "1100" & x"0";
    report oi(ob);
    sw <= a & ws;
    si <= ia'(1, 1) & x;
    sp <= p & pt'(8, true);
    wait for 1 ns;
    report "sig " & img(sw) & " " & img(si) & " " & img(sp);
    sw <= sw(2 to 3) & sw(0 to 1);
    wait for 1 ns;
    report "sig " & img(sw);
    wait;
  end process;
end;
