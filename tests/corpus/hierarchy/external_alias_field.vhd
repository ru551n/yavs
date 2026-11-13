-- difftest: top=external_alias_field std=2008 stop=100ns ignore=order
-- ignore=order: processes of different branches report in the same cycle, in an order the LRM leaves open.
-- Aliases of a record field of an external name (LRM 6.6.2, 8.7) in selected and unselected
-- branches of if-, case- and for-generates, in a block and in a process. A branch the generics
-- do not select is not elaborated (LRM 14.2): the external names of the aliases in it are never
-- resolved, and some of them name nothing (`.nowhere`).
package pk is
  type in_t is record
    x : integer;
    y : bit;
  end record;
  type rec_t is record
    a : bit_vector(3 downto 0);
    n : integer;
    i : in_t;
  end record;
end package;

use work.pk.all;
entity sub is end;
architecture a of sub is
  signal r : rec_t := (a => "0000", n => 0, i => (x => 0, y => '0'));
  constant c : rec_t := (a => "1010", n => 7, i => (x => 8, y => '1'));
begin
  r.a <= "0110" after 2 ns;
  r.n <= 5 after 2 ns, 6 after 4 ns;
  r.i.x <= 3 after 3 ns;
  r.i.y <= '1' after 3 ns;
end;

use work.pk.all;
entity external_alias_field is
  generic (V : natural := 2; W : natural := 0; N : natural := 0; M : natural := 2);
end;
architecture tb of external_alias_field is
begin
  u : entity work.sub;

  -- g1 is not selected, g2 is; g3 selects its else branch.
  g1 : if V = 1 generate
    alias fn is <<signal .external_alias_field.nowhere : rec_t>>.n;
    alias fi is <<signal .external_alias_field.u.r : rec_t>>.i.x;
    alias fk is <<constant .external_alias_field.u.c : rec_t>>.i;
  begin
    p : process
      alias pa is <<signal .external_alias_field.u.r : rec_t>>.a;
    begin
      wait on fn, fi, pa;
      report "g1 " & integer'image(fn + fi + fk.x) & to_string(pa);
      wait;
    end process;
    b : block
      alias bl is <<signal .external_alias_field.u.r : rec_t>>.i;
    begin
      process (bl) begin report "g1 b " & integer'image(bl.x); end process;
    end block;
  end generate;
  g2 : if V = 2 generate
    alias fn is <<signal .external_alias_field.u.r : rec_t>>.n;
    alias fi is <<signal .external_alias_field.u.r : rec_t>>.i.x;
    alias fk is <<constant .external_alias_field.u.c : rec_t>>.i;
  begin
    p : process
      alias pa is <<signal .external_alias_field.u.r : rec_t>>.a;
    begin
      wait for 1 ns;
      report "g2 " & integer'image(fn + fi + fk.x) & " " & to_string(pa);
      wait on fn, fi, pa;
      report "g2 " & integer'image(fn + fi + fk.x) & " " & to_string(pa);
      wait;
    end process;
    b : block
      alias bl is <<signal .external_alias_field.u.r : rec_t>>.i;
    begin
      process (bl) begin report "g2 b " & integer'image(bl.x) & bit'image(bl.y); end process;
    end block;
  end generate;
  g3 : if V = 3 generate
    alias fn is <<signal .external_alias_field.nowhere : rec_t>>.n;
  begin
    process (all) begin report "g3 " & integer'image(fn); end process;
  else generate
    alias fn is <<signal .external_alias_field.u.r : rec_t>>.n;
  begin
    process (all) begin report "g3 else " & integer'image(fn); end process;
  end generate;

  -- W = 0 selects the first alternative.
  gc : case W generate
    when 0 =>
      alias fy is <<signal .external_alias_field.u.r : rec_t>>.i.y;
    begin
      process (fy) begin report "gc 0 " & bit'image(fy); end process;
    when 1 =>
      alias fy is <<signal .external_alias_field.nowhere : rec_t>>.i.y;
    begin
      process (fy) begin report "gc 1 " & bit'image(fy); end process;
    when others =>
      alias fn is <<signal .external_alias_field.u.r : rec_t>>.n;
    begin
      process (all) begin report "gc others " & integer'image(fn); end process;
  end generate;

  -- N = 0 never elaborates the body, M = 2 does it twice.
  gf0 : for i in 1 to N generate
    alias fn is <<signal .external_alias_field.nowhere : rec_t>>.n;
  begin
    process (fn) begin report "gf0 " & integer'image(fn); end process;
  end generate;
  gf : for i in 0 to M - 1 generate
    alias fi is <<signal .external_alias_field.u.r : rec_t>>.i;
  begin
    process (fi) begin report "gf " & integer'image(i) & " " & integer'image(fi.x); end process;
  end generate;
end;
