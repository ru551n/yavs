-- difftest: top=callout_report std=2008
-- Reports of a resolution function and of a port conversion function made during the run
-- carry the time and delta of the cycle that called them, not "(init)" (#372).
entity callout_report is end;
architecture a of callout_report is
  type int_vec is array (natural range <>) of integer;
  function sumres(v : int_vec) return integer is
    variable m : integer := 0;
  begin
    for i in v'range loop m := m + v(i); end loop;
    if m /= integer'low then report "resolve " & integer'image(m); end if;
    return m;
  end;
  function to_int(b : bit) return integer is
  begin
    if b = '1' then report "convert " & bit'image(b); end if;
    return bit'pos(b);
  end;
  subtype rint is sumres integer;
  signal r : rint;
  signal s : bit;
begin
  b : block is
    port (p : in integer);
    port map (p => to_int(s));
  begin
  end block;
  process
  begin
    s <= '1' after 1 ns;
    r <= 3 after 2 ns;
    wait for 3 ns;
    assert r = 3 severity failure;
    wait;
  end process;
end;
