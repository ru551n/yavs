-- difftest: top=generic_entities_2019 std=2019
-- Entities with generic types, subprograms and packages: one instance per map, nested
-- generic entities, instances in a for generate, class operations of a formal type.
package lim_pkg is
  generic (n : natural);
  function lim return natural;
end;
package body lim_pkg is
  function lim return natural is begin return n; end;
end;

entity cell is
  generic (type t; init : t; function nxt(x : t) return t);
  port (clk : in bit; q : out t);
end;
architecture a of cell is
  signal s : t := init;
begin
  q <= s;
  process (clk) begin
    if clk'event and clk = '1' then
      s <= nxt(s);
    end if;
  end process;
end;

entity pair is
  generic (type e; function img(x : e) return string; a0, b0 : e;
           function step(x : e) return e; dly : time);
  port (clk : in bit);
end;
architecture a of pair is
  signal qa, qb : e;
  signal same : natural := 0;
  -- A local function as the actual: the reference simulator rejects a formal one.
  function nx(x : e) return e is begin return step(x); end;
begin
  ua : entity work.cell generic map (t => e, init => a0, nxt => nx) port map (clk, qa);
  ub : entity work.cell generic map (t => e, init => b0, nxt => nx) port map (clk, qb);
  process (qa, qb) begin
    if qa = qb then
      same <= same + 1;
    end if;
  end process;
  process begin
    wait for dly;
    report img(qa) & " " & img(qb) & " same=" & integer'image(same);
    wait;
  end process;
end;

entity lim_user is
  generic (package p is new work.lim_pkg generic map (<>); dly : time);
end;
architecture a of lim_user is
  use p.all;
begin
  process begin
    wait for dly;
    report "lim " & integer'image(lim);
    wait;
  end process;
end;

entity generic_entities_2019 is end;
architecture t of generic_entities_2019 is
  package p3 is new work.lim_pkg generic map (n => 3);
  package p5 is new work.lim_pkg generic map (n => 5);
  function inc(x : integer) return integer is begin return x + 1; end;
  function ii(x : integer) return string is begin return integer'image(x); end;
  function rot(x : character) return character is
  begin
    if x = 'z' then return 'a'; else return character'succ(x); end if;
  end;
  function ci(x : character) return string is begin return (1 => x); end;
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 40 ns;
  g : for i in 0 to 1 generate
    pi : entity work.pair
      generic map (e => integer, img => ii, a0 => i, b0 => 2 * i, step => inc, dly => (i + 1) * 13 ns)
      port map (clk);
  end generate;
  pc : entity work.pair
    generic map (e => character, img => ci, a0 => 'x', b0 => 'y', step => rot, dly => 47 ns)
    port map (clk);
  l3 : entity work.lim_user generic map (p => p3, dly => 1 ns);
  l5 : entity work.lim_user generic map (p => p5, dly => 2 ns);
end;
