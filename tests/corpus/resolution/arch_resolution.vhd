-- difftest: top=arch_resolution std=2008
-- A resolution function declared in an architecture reads that instance's constant
-- (computed from a generic), so each instance resolves with its own data (#164).
entity arch_resolution_sub is generic (bias : integer); port (o : out integer); end;
architecture a of arch_resolution_sub is
  type int_vec is array (natural range <>) of integer;
  constant offs : integer := bias * 100;
  function sumres(v : int_vec) return integer is
    variable m : integer := offs;
  begin
    for i in v'range loop m := m + v(i); end loop;
    return m;
  end;
  subtype rint is sumres integer;
  signal s : rint := 0;
begin
  s <= 3 after 1 ns;
  s <= 4 after 2 ns;
  o <= s;
end;
entity arch_resolution is end;
architecture t of arch_resolution is
  signal o1, o2 : integer;
begin
  u1 : entity work.arch_resolution_sub generic map (1) port map (o1);
  u2 : entity work.arch_resolution_sub generic map (2) port map (o2);
  process (o1, o2) begin
    report "o1=" & integer'image(o1) & " o2=" & integer'image(o2);
  end process;
end;
