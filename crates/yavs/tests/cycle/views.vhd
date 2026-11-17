-- Cycle mode: procedure calls with mode views (VHDL-2019). A view reads its `in` and `inout`
-- elements and drives its `out` ones; the subset checker counted neither, so a concurrent call
-- driving only through a view had no outputs and the logic reading them saw the value of the
-- previous wake. Also a nested view, a converse view, an alias as the actual, calls in clocked
-- and combinational processes, and a procedure that may return before assigning its `out`.
library ieee;
use ieee.std_logic_1164.all;
package cycle_views_pkg is
  generic (W : positive);
  type sub_t is record
    req : std_ulogic_vector(W - 1 downto 0);
    ack : std_ulogic;
  end record;
  view sub_m of sub_t is
    req : out;
    ack : in;
  end view;
  type bus_t is record
    a : std_ulogic_vector(W - 1 downto 0);
    b : std_ulogic_vector(W - 1 downto 0);
    s : sub_t;
    c : std_ulogic_vector(W - 1 downto 0);
  end record;
  view bus_m of bus_t is
    a : in;
    b : out;
    s : view sub_m;
    c : in;
  end view;
  type pair_t is record
    x : std_ulogic_vector(W - 1 downto 0);
    y : std_ulogic_vector(W - 1 downto 0);
  end record;
  view out_m of pair_t is
    x : out;
    y : out;
  end view;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_views is
  generic (W : positive := 8);
end entity;
architecture a of cycle_views is
  package p is new work.cycle_views_pkg generic map (W => W);
  use p.all;
  constant Z : std_ulogic_vector(W - 1 downto 0) := (others => '0');

  -- Drives only through its view.
  procedure split (signal d : in std_ulogic_vector(W - 1 downto 0); signal o : view out_m) is
  begin
    o.x <= d xor (d'range => '1');
    o.y <= d(0) & d(W - 1 downto 1);
  end procedure;
  -- A nested view.
  procedure drive (signal m : view bus_m; signal y : out std_ulogic_vector(W - 1 downto 0)) is
  begin
    m.b <= m.a xor m.c;
    m.s.req <= m.a and (m.a'range => m.s.ack);
    y <= m.c;
  end procedure;
  -- The converse view.
  procedure resp (signal m : view bus_m'converse) is
  begin
    m.a <= m.b(W - 2 downto 0) & m.s.req(W - 1) when W > 1 else m.s.req;
    m.s.ack <= m.s.req(0);
    m.c <= not m.b;
  end procedure;
  -- May return before it assigns `s`: a latch, not a member.
  procedure early (signal a : in std_ulogic; signal d : in std_ulogic_vector(W - 1 downto 0);
                   signal s : out std_ulogic_vector(W - 1 downto 0)) is
  begin
    if a = '1' then
      return;
    end if;
    s <= d;
  end procedure;

  signal clk : std_ulogic := '0';
  signal d : unsigned(W - 1 downto 0) := (others => '0');
  signal dv : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal pr : pair_t := (Z, Z);
  signal b1, b2, b3 : bus_t := (Z, Z, (Z, '0'), Z);
  alias b2a is b2;
  signal po, bo, ba, bv, bz, lt : std_ulogic_vector(W - 1 downto 0) := Z;
begin
  clk <= not clk after 5 ns;

  -- Readers before the call that drives what they read.
  po <= pr.x xor pr.y;
  bz <= b1.b xor b2.s.req xor b3.c;
  dv <= std_ulogic_vector(d);
  outs : split(dv, pr);
  c1 : drive(b1, bo);
  c2 : drive(b2a, ba);
  clocked : process (clk)
  begin
    if rising_edge(clk) then
      resp(b1);
      resp(b2);
      resp(b3);
    end if;
  end process;
  comb : process (all)
  begin
    drive(b3, bv);
  end process;
  latch : process (all)
  begin
    early(dv(0), dv, lt);
  end process;

  stim : process
  begin
    for i in 0 to 23 loop
      wait until falling_edge(clk);
      d <= d + 37 when i mod 3 = 0 else not d;
      wait for 1 ns;
      report "po=" & to_hstring(po) & " x=" & to_hstring(pr.x) & " bo=" & to_hstring(bo)
        & " ba=" & to_hstring(ba) & " bv=" & to_hstring(bv) & " bz=" & to_hstring(bz)
        & " lt=" & to_hstring(lt);
    end loop;
    std.env.finish;
  end process;
end architecture;
