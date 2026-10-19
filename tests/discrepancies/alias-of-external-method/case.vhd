-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.2, 8.7: aliases of external signals, constants and variables
package p is
  type counter is protected
    procedure inc;
  end protected;
end package;
package body p is
  type counter is protected body
    variable n : natural := 0;
    procedure inc is begin n := n + 1; end procedure;
  end protected body;
end package body;
entity sub is end entity;
architecture r of sub is
  signal s : bit_vector(3 downto 0);
  constant k : integer := 3;
  shared variable sv : work.p.counter;
begin
end architecture;
entity e is end entity;
architecture a of e is
  signal t : bit;
  signal n : integer;
begin
  u : entity work.sub;
  b : block
    alias s is <<signal .e.u.s : bit_vector(3 downto 0)>>;
  begin
    t <= s(0);
  end block;
  process
    alias s is <<signal .e.u.s : bit_vector>>;
    alias k is <<constant .e.u.k : integer>>;
    alias v is <<variable .e.u.sv : work.p.counter>>;
  begin
    s <= "0101";
    n <= k + s'length;
    v.inc;
    wait;
  end process;
end architecture;
