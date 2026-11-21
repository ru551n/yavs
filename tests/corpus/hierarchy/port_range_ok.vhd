-- difftest: top=port_range_ok std=2008
-- Ports whose actual's subtype differs from theirs, every value in range (#1183): out, in,
-- inout and buffer ports, conversions both ways, a component port, an element actual and a
-- record field. Nothing fails, the values arrive unchanged.
entity port_range_ok_cell is
  port (
    i  : in natural;
    o  : out integer := 1;
    b  : inout integer := 0;
    bf : buffer integer := 2;
    oc : out integer := 0);
end entity;
architecture a of port_range_ok_cell is
begin
  process
  begin
    wait on i;
    o <= i + 1;
    b <= i + 2;
    bf <= i + 3;
    oc <= i;
    wait on i;
    report "cell i=" & integer'image(i) & " b=" & integer'image(b) & " bf=" & integer'image(bf);
    wait;
  end process;
end architecture;

entity port_range_ok is end entity;
architecture a of port_range_ok is
  component port_range_ok_cell is
    port (
      i  : in integer range 0 to 5;
      o  : out integer range 0 to 9;
      b  : inout integer;
      bf : buffer integer;
      oc : out integer);
  end component;
  type digits is array (0 to 1) of integer range 0 to 9;
  type pair is record
    lo : natural;
    hi : integer range 0 to 9;
  end record;
  function half (x : integer) return integer is begin return x / 2; end function;
  signal si : integer := 0;
  signal so : digits := (others => 0);
  signal sb : natural := 0;
  signal sp : pair := (0, 0);
  signal sc : integer range 0 to 3 := 0;
begin
  u : port_range_ok_cell port map (i => si, o => so(1), b => sb, bf => sp.hi, half(oc) => sc);
  v : entity work.port_range_ok_cell port map (i => half(si), o => open, b => open, bf => open, oc => open);
  process
  begin
    wait for 1 ns;
    si <= 4;
    wait for 1 ns;
    report "so(1)=" & integer'image(so(1)) & " sb=" & integer'image(sb) & " hi=" & integer'image(sp.hi) & " sc=" & integer'image(sc);
    si <= 5;
    wait;
  end process;
end architecture;
