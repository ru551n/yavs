-- Scalar `out` and `inout` formals of procedures called by member processes, as the normal
-- mode checks them (#1181, #1175): a value driven through a signal formal against the formal's
-- subtype and then its actual's, a signal actual never at the call, a variable actual at the
-- call (`inout`) and when copied back, its element's place fixed at the call (#1182); a range
-- that reads a variable keeps the caller out (its value at the declaration).
package sr_pkg is
  type st is (s0, s1, s2, s3);
  type ia is array (0 to 3) of integer range 0 to 9;
  type rec is record
    f : integer range 0 to 9;
    g : integer;
  end record;
  procedure drv (signal o : out integer; v : integer);
  procedure drvn (signal o : out natural; v : integer);
  procedure drve (signal o : out st; v : st);
  procedure drvc (signal o : out integer; v : integer; c : boolean);
  procedure rd (signal i : in natural; signal o : out integer);
  procedure fwd (signal o : out integer range 10 to 20; v : integer);
  procedure nest (signal o : out integer; v : integer);
  procedure setv (o : out integer; v : integer);
  procedure sete (o : out st; v : st);
  procedure incv (o : inout natural);
  procedure twice (o : out integer; v : integer);
  procedure nxt (o : out integer; i : inout integer);
  function fin (signal i : natural) return integer;
end package;
package body sr_pkg is
  procedure drv (signal o : out integer; v : integer) is
  begin
    o <= v;
  end procedure;
  procedure drvn (signal o : out natural; v : integer) is
  begin
    o <= v;
  end procedure;
  procedure drve (signal o : out st; v : st) is
  begin
    o <= v;
  end procedure;
  procedure drvc (signal o : out integer; v : integer; c : boolean) is
  begin
    o <= v when c else 0;
  end procedure;
  procedure rd (signal i : in natural; signal o : out integer) is
  begin
    o <= i;
  end procedure;
  procedure fwd (signal o : out integer range 10 to 20; v : integer) is
  begin
    drv(o, v);
  end procedure;
  -- A body that declares a subprogram: the normal mode's plain body, no check of the actual.
  procedure nest (signal o : out integer; v : integer) is
    function id (x : integer) return integer is
    begin
      return x;
    end function;
  begin
    o <= v;
  end procedure;
  procedure setv (o : out integer; v : integer) is
  begin
    o := v;
  end procedure;
  procedure sete (o : out st; v : st) is
  begin
    o := v;
  end procedure;
  procedure incv (o : inout natural) is
  begin
    o := o + 1;
  end procedure;
  procedure twice (o : out integer; v : integer) is
  begin
    o := v + 10;
    o := v;
  end procedure;
  procedure nxt (o : out integer; i : inout integer) is
  begin
    i := i + 1;
    o := 7;
  end procedure;
  function fin (signal i : natural) return integer is
  begin
    return i;
  end function;
end package body;

-- The testbench of every design: `go` takes `first`, then `second` at 1 ns.
entity sr_tb is
  generic (first, second : integer);
  port (go : out integer := 0; s : in integer);
end entity;
architecture a of sr_tb is
begin
  process
  begin
    go <= first;
    wait for 1 ns;
    report "s=" & integer'image(s);
    go <= second;
    wait for 1 ns;
    report "s=" & integer'image(s);
    wait;
  end process;
end architecture;

use work.sr_pkg.all;
entity sr_sig is
end entity;
architecture a of sr_sig is
  signal s : integer range 0 to 9 := 0;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    drv(s, go);
  end process;
  tb : entity work.sr_tb generic map (3, 10) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_sigconc is
end entity;
architecture a of sr_sigconc is
  signal s : integer range 0 to 9 := 0;
  signal go : integer := 0;
begin
  c : drv(s, go);
  tb : entity work.sr_tb generic map (3, 10) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_sigf is
end entity;
architecture a of sr_sigf is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    drvn(s, go);
  end process;
  tb : entity work.sr_tb generic map (3, -1) port map (go, s);
end architecture;

-- The actual holds a value outside the formal's subtype: never checked at the call.
use work.sr_pkg.all;
entity sr_sigok is
end entity;
architecture a of sr_sigok is
  signal s : integer := -1;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    drvn(s, go);
  end process;
  tb : entity work.sr_tb generic map (3, 4) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_sigenum is
end entity;
architecture a of sr_sigenum is
  subtype sub is st range s0 to s1;
  signal s : sub := s0;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    drve(s, st'val(go));
  end process;
  tb : entity work.sr_tb generic map (1, 2) port map (go, st'pos(s));
end architecture;

use work.sr_pkg.all;
entity sr_sigelem is
end entity;
architecture a of sr_sigelem is
  signal sa : ia := (others => 0);
  signal go : integer := 0;
begin
  c : process (go)
  begin
    drv(sa(1), go);
  end process;
  tb : entity work.sr_tb generic map (3, 10) port map (go, sa(1));
end architecture;

use work.sr_pkg.all;
entity sr_sigfield is
end entity;
architecture a of sr_sigfield is
  signal r : rec := (0, 0);
  signal go : integer := 0;
begin
  c : process (go)
  begin
    drv(r.f, go);
  end process;
  tb : entity work.sr_tb generic map (3, 10) port map (go, r.f);
end architecture;

-- A clocked caller and a conditional drive.
use work.sr_pkg.all;
entity sr_sigclk is
end entity;
architecture a of sr_sigclk is
  signal s : integer range 0 to 9 := 0;
  signal go : integer := 0;
  signal clk : bit := '0';
begin
  ck : process
  begin
    for i in 1 to 12 loop
      clk <= not clk;
      wait for 250 ps;
    end loop;
    wait;
  end process;
  c : process (clk)
  begin
    if clk'event and clk = '1' then
      drvc(s, go, go > 0);
    end if;
  end process;
  tb : entity work.sr_tb generic map (3, 10) port map (go, s);
end architecture;

-- A formal passed on: its actual's bounds narrowed to its subtype (10 to 15, not 0 to 15).
use work.sr_pkg.all;
entity sr_fwd is
end entity;
architecture a of sr_fwd is
  signal s : integer range 0 to 15 := 12;
  signal go : integer := 0;
  signal clk : bit := '0';
begin
  ck : process
  begin
    for i in 1 to 12 loop
      clk <= not clk;
      wait for 250 ps;
    end loop;
    wait;
  end process;
  c : process (clk)
  begin
    if clk'event and clk = '1' then
      fwd(s, go);
    end if;
  end process;
  tb : entity work.sr_tb generic map (12, 5) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_sigin is
end entity;
architecture a of sr_sigin is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    rd(go, s);
  end process;
  tb : entity work.sr_tb generic map (3, -1) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_fin is
end entity;
architecture a of sr_fin is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    s <= fin(go);
  end process;
  tb : entity work.sr_tb generic map (3, -1) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_nested is
end entity;
architecture a of sr_nested is
  signal s : integer range 0 to 9 := 0;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    nest(s, go);
  end process;
  tb : entity work.sr_tb generic map (3, 10) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_varout is
end entity;
architecture a of sr_varout is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    variable v : integer range 0 to 9;
  begin
    setv(v, go);
    s <= v;
  end process;
  tb : entity work.sr_tb generic map (3, 10) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_varenum is
end entity;
architecture a of sr_varenum is
  subtype sub is st range s0 to s1;
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    variable v : sub;
  begin
    sete(v, st'val(go));
    s <= st'pos(v);
  end process;
  tb : entity work.sr_tb generic map (1, 2) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_inout is
end entity;
architecture a of sr_inout is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    variable v : integer;
  begin
    v := go;
    incv(v);
    s <= v;
  end process;
  tb : entity work.sr_tb generic map (3, -5) port map (go, s);
end architecture;

-- A value outside the actual's subtype that the formal holds only during the call.
use work.sr_pkg.all;
entity sr_twice is
end entity;
architecture a of sr_twice is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    variable v : integer range 0 to 9;
  begin
    twice(v, go);
    s <= v;
  end process;
  tb : entity work.sr_tb generic map (3, 4) port map (go, s);
end architecture;

use work.sr_pkg.all;
entity sr_index is
end entity;
architecture a of sr_index is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    type iv is array (0 to 3) of integer;
    variable b : iv;
    variable i : integer;
  begin
    b := (others => 0);
    i := go;
    nxt(b(i), i);
    s <= b(0) * 1000 + b(1) * 100 + b(2) * 10 + b(3);
  end process;
  tb : entity work.sr_tb generic map (1, 2) port map (go, s);
end architecture;

entity sr_frange is
end entity;
architecture a of sr_frange is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    variable k : integer := 5;
    variable v : integer;
    procedure bump (x : inout integer range 0 to k) is
    begin
      x := x + 1;
    end procedure;
  begin
    k := 100;
    v := go;
    bump(v);
    s <= v;
  end process;
  tb : entity work.sr_tb generic map (3, 7) port map (go, s);
end architecture;

entity sr_subrange is
end entity;
architecture a of sr_subrange is
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    variable k : integer := 5;
    subtype t is integer range 0 to k;
    procedure bump (x : inout integer) is
      variable z : t;
    begin
      z := x + 1;
      x := z;
    end procedure;
    variable v : integer;
  begin
    k := 100;
    v := go;
    bump(v);
    s <= v;
  end process;
  tb : entity work.sr_tb generic map (3, 7) port map (go, s);
end architecture;

-- An enumeration value and index outside their subtypes: the message shows literals (#1178).
entity sr_enumvar is
end entity;
architecture a of sr_enumvar is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
    variable v : rgb;
  begin
    v := color'val(go);
    s <= color'pos(v);
  end process;
  tb : entity work.sr_tb generic map (1, 3) port map (go, s);
end architecture;

entity sr_enumix is
end entity;
architecture a of sr_enumix is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  type tab is array (rgb) of integer;
  constant t : tab := (10, 20, 30);
  signal s : integer := 0;
  signal go : integer := 0;
begin
  c : process (go)
  begin
    s <= t(color'val(go));
  end process;
  tb : entity work.sr_tb generic map (1, 3) port map (go, s);
end architecture;
