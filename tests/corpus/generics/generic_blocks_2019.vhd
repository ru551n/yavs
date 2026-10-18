-- difftest: top=generic_blocks_cfg std=2019
-- Blocks with generic types, subprograms and packages (one generic map each, nested, in a
-- for generate, with the enclosing entity's generic type as the actual), and
-- configuration specifications inside the architecture of a generic-type entity; a
-- configuration declaration binding an instance inside a generic block.
package scale_pkg is
  generic (m : integer);
  function scale(x : integer) return integer;
end;
package body scale_pkg is
  function scale(x : integer) return integer is begin return x * m; end;
end;

package scale10 is new work.scale_pkg generic map (m => 10);

entity tap is
  generic (n : integer);
  port (y : out integer);
end;
architecture plus of tap is
begin
  y <= n + 1000;
end;
architecture minus of tap is
begin
  y <= n - 1000;
end;

entity holder is
  generic (type t; init : t; function nx(x : t) return t;
           function img(x : t) return string; tag : string);
end;
architecture a of holder is
  component tap is
    generic (n : integer);
    port (y : out integer);
  end component;
  for u1 : tap use entity work.tap(minus);
  signal y1 : integer;
  -- Local functions as the actuals: the reference simulators reject a formal one.
  function lnx(x : t) return t is begin return nx(x); end;
  function limg(x : t) return string is begin return img(x); end;
begin
  u1 : tap generic map (n => 1) port map (y1);
  g : for i in 1 to 2 generate
    for u2 : tap use entity work.tap(plus);
    signal y2 : integer;
  begin
    u2 : tap generic map (n => i) port map (y2);
    -- The entity's generic type as the actual of the block's.
    b : block
      generic (type bt; b0 : bt; function bn(x : bt) return bt;
               function bi(x : bt) return string);
      generic map (bt => t, b0 => init, bn => lnx, bi => limg);
      signal s : bt := b0;
    begin
      process begin
        for k in 1 to i loop
          s <= bn(s);
          wait for 1 ns + tag'length * 10 ps;
        end loop;
        report tag & " " & integer'image(i) & ": " & bi(s) & " y2=" & integer'image(y2);
        wait;
      end process;
    end block;
  end generate;
  process begin
    wait for 4 ns + tag'length * 100 ps;
    report tag & " y1=" & integer'image(y1);
    wait;
  end process;
end;

entity generic_blocks is end;
architecture a of generic_blocks is
  component tap is
    generic (n : integer);
    port (y : out integer);
  end component;
  function inc(x : integer) return integer is begin return x + 1; end;
  function iimg(x : integer) return string is begin return integer'image(x); end;
  function shl(x : bit_vector) return bit_vector is
  begin
    return x(x'left + 1 to x'right) & '1';
  end;
  function bimg(x : bit_vector) return string is begin return to_string(x); end;
begin
  hi : entity work.holder
    generic map (t => integer, init => 7, nx => inc, img => iimg, tag => "int");
  hb : entity work.holder
    generic map (t => bit_vector(0 to 3), init => "0000", nx => shl, img => bimg, tag => "bv");

  outer : block
    generic (type ot; o0 : ot; function onx(x : ot) return ot;
             function oi(x : ot) return string);
    generic map (ot => integer, o0 => 40, onx => inc, oi => iimg);
    constant c : ot := onx(o0);
    function lon(x : ot) return ot is begin return onx(x); end;
    function loi(x : ot) return string is begin return oi(x); end;
  begin
    inner : block
      generic (type it; i0 : it; function inn(x : it) return it;
               function ii(x : it) return string);
      generic map (it => ot, i0 => c, inn => lon, ii => loi);
    begin
      process begin
        wait for 6 ns;
        report "nested " & ii(inn(i0));
        wait;
      end process;
    end block;
  end block;

  pk : block
    generic (package p is new work.scale_pkg generic map (<>); v : integer);
    generic map (p => work.scale10, v => 4);
    use p.all;
  begin
    process begin
      wait for 7 ns;
      report "scaled " & integer'image(scale(v));
      assert scale(v) = 40 severity failure;
      wait;
    end process;
  end block;
  cb : block
    generic (type ct; function ci(x : ct) return string);
    generic map (ct => integer, ci => iimg);
    signal y : integer;
  begin
    u : tap generic map (n => 5) port map (y);
    process begin
      wait for 8 ns;
      report "configured " & integer'image(y);
      wait;
    end process;
  end block;
end;

configuration generic_blocks_cfg of generic_blocks is
  for a
    for cb
      for u : tap use entity work.tap(minus);
      end for;
    end for;
  end for;
end;
