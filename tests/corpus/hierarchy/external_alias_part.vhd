-- difftest: top=external_alias_part std=2008 stop=100ns ignore=order,delta
-- ignore=order: processes of different branches report in the same cycle, in an order the LRM leaves open.
-- ignore=delta: sim1 takes a delta cycle for the port actual that is an external name or a part of one, yavs none (L49).
-- Aliases of a part of an external name (LRM 6.6.2, 8.7): a slice, an element, a record field and
-- nested parts of signals and of a constant, declared in generate branches, blocks, processes
-- and subprograms. A branch the generics do not select is not elaborated (LRM 14.2): its
-- external names are never resolved, and one of them names nothing at all (`.nowhere`).
package pk is
  type rec_t is record
    a : bit_vector(3 downto 0);
    n : integer;
  end record;
  type rec_arr is array (0 to 1) of rec_t;
  type int_arr is array (0 to 2) of integer;
end package;

use work.pk.all;
entity sub is end;
architecture a of sub is
  signal sv : bit_vector(7 downto 0) := x"00";
  signal r : rec_t := (a => "0000", n => 0);
  signal ra : rec_arr := (others => (a => "0000", n => 0));
  constant cv : int_arr := (10, 20, 30);
begin
  sv <= x"A5" after 1 ns, x"3C" after 4 ns;
  r.a <= "0110" after 2 ns;
  r.n <= 5 after 2 ns;
  ra(1).a <= "1001" after 3 ns;
  ra(1).n <= 9 after 3 ns;
end;

entity probe is
  port (i : in bit; tag : in string);
end;
architecture a of probe is
begin
  process (i) begin report "probe " & tag & " " & bit'image(i); end process;
end;

use work.pk.all;
entity external_alias_part is
  generic (V : natural := 2; W : natural := 0; N : natural := 0; M : natural := 2);
end;
architecture tb of external_alias_part is
  signal o, o2, o3 : bit;
begin
  u : entity work.sub;

  -- if-generate: g1 and g3 are not selected, g2 is.
  g1 : if V = 1 generate
    alias sl is <<signal .external_alias_part.nowhere : bit_vector(7 downto 0)>>(3 downto 0);
    alias el is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(1);
    alias fn is <<signal .external_alias_part.u.r : rec_t>>.n;
    alias ne is <<signal .external_alias_part.u.ra : rec_arr>>(1).a(3);
    alias kc is <<constant .external_alias_part.u.cv : int_arr>>(2);
  begin
    p : process
      alias pa is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(7 downto 4);
      impure function f return integer is
        alias fa is <<signal .external_alias_part.u.r : rec_t>>.n;
      begin
        return fa;
      end function;
    begin
      wait on sl, el, fn, ne, pa;
      report "g1 " & to_string(sl) & bit'image(el) & integer'image(fn + kc + f) & to_string(pa);
      wait;
    end process;
    o <= el;
    -- direct uses of parts, in a concurrent assignment, process (all), a signal's initial value
    -- and the expression actual of a port
    o2 <= <<signal .external_alias_part.nowhere : bit_vector(7 downto 0)>>(2);
    process begin
      wait on <<signal .external_alias_part.nowhere : rec_t>>.n;
      report "g1 wait " & integer'image(<<signal .external_alias_part.nowhere : rec_t>>.n
        + <<constant .external_alias_part.nowhere : int_arr>>(1));
    end process;
    pr : entity work.probe port map (i => <<signal .external_alias_part.nowhere : bit_vector(7 downto 0)>>(3), tag => "g1");
    gb : block
      signal q : bit := <<signal .external_alias_part.nowhere : bit_vector(7 downto 0)>>(1);
    begin
      process (q) begin report "g1 gb " & bit'image(q); end process;
    end block;
    gn : for k in 0 to 1 generate
      alias kb is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(k);
    begin
      process (kb) begin report "g1 gn " & bit'image(kb); end process;
    end generate;
    b : block
      alias bl is <<signal .external_alias_part.u.ra : rec_arr>>(0);
    begin
      process begin wait on bl.a; report "g1 b " & to_string(bl.a); wait; end process;
    end block;
  end generate;
  g2 : if V = 2 generate
    alias sl is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(3 downto 0);
    alias el is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(1);
    alias fn is <<signal .external_alias_part.u.r : rec_t>>.n;
    alias ne is <<signal .external_alias_part.u.ra : rec_arr>>(1).a(3);
    alias kc is <<constant .external_alias_part.u.cv : int_arr>>(2);
  begin
    p : process
      alias pa is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(7 downto 4);
      impure function f return integer is
        alias fa is <<signal .external_alias_part.u.r : rec_t>>.n;
      begin
        return fa;
      end function;
    begin
      wait for 1.5 ns;
      report "g2 " & to_string(sl) & " " & bit'image(el) & " " & integer'image(fn + kc + f) & " " & to_string(pa);
      wait for 1 ns;
      report "g2 " & integer'image(fn + kc + f) & " " & to_string(<<signal .external_alias_part.u.r : rec_t>>.a);
      wait on ne;
      report "g2 ne " & bit'image(ne) & " " & integer'image(kc);
      wait;
    end process;
    o <= <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(2);
    o2 <= <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(7);
    process begin
      wait on <<signal .external_alias_part.u.r : rec_t>>.n;
      report "g2 wait " & integer'image(<<signal .external_alias_part.u.r : rec_t>>.n
        + <<constant .external_alias_part.u.cv : int_arr>>(1));
    end process;
    pr : entity work.probe port map (i => <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(3), tag => "g2");
    b : block
      alias bl is <<signal .external_alias_part.u.ra : rec_arr>>(1);
    begin
      process begin wait on bl.a; report "g2 b " & to_string(bl.a); wait; end process;
    end block;
  end generate;
  g3 : if V = 3 generate
    alias sl is <<signal .external_alias_part.nowhere : bit_vector(7 downto 0)>>(3 downto 0);
  begin
    process (all) begin report "g3 " & to_string(sl); end process;
  else generate
    alias el is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(0);
  begin
    process (all) begin report "g3 else " & bit'image(el); end process;
  end generate;

  -- case-generate: W = 0 selects the first alternative.
  gc : case W generate
    when 0 =>
      alias sl is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(7 downto 6);
    begin
      process (sl) begin report "gc 0 " & to_string(sl); end process;
    when 1 =>
      alias sl is <<signal .external_alias_part.nowhere : bit_vector(7 downto 0)>>(7 downto 6);
    begin
      process (sl) begin report "gc 1 " & to_string(sl); end process;
    when others =>
      alias fn is <<signal .external_alias_part.u.r : rec_t>>.n;
    begin
      process (all) begin report "gc others " & integer'image(fn); end process;
  end generate;

  -- for-generate: N = 0 never elaborates its body, M = 2 does it twice.
  gf0 : for i in 1 to N generate
    alias sl is <<signal .external_alias_part.nowhere : bit_vector(7 downto 0)>>(3 downto 0);
  begin
    process (sl) begin report "gf0 " & to_string(sl); end process;
  end generate;
  gf : for i in 0 to M - 1 generate
    alias nib is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(4 * i + 3 downto 4 * i);
    alias raa is <<signal .external_alias_part.u.ra : rec_arr>>(i);
  begin
    process (nib) begin report "gf " & integer'image(i) & " " & to_string(nib); end process;
    process (raa.n) begin report "gf " & integer'image(i) & " n " & integer'image(raa.n); end process;
  end generate;

  tb : block
    alias top_sl is <<signal .external_alias_part.u.sv : bit_vector(7 downto 0)>>(7 downto 6);
  begin
    process begin wait for 21 ns; report "top_sl " & to_string(top_sl); wait; end process;
  end block;

  chk : process
  begin
    wait for 20 ns;
    assert o = '1' and o2 = '0' report "o" severity failure;
    report "done V=" & integer'image(V);
    wait;
  end process;
end;
