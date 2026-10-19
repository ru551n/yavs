-- difftest: top=external_port_forms std=2008 stop=20ns ref=hand ignore=order
-- ignore=order: the monitors report in the same cycle.
-- ref=hand: LRM 8.7 takes a relative path from the region that contains the port map, here
-- fp_pair; the default reference simulator takes it from the instantiated entity and rejects
-- `l.hidden`. The expected output is its output for the same design with `^.l.hidden`.
-- External names as port actuals (LRM 8.7): relative (^), through a generate index and a
-- package signal (@). The monitors sample at fixed times, so the result does not depend on
-- the delta at which the port follows the external signal.
package fp_pkg is
  signal pk : natural := 1;
end package;
entity fp_leaf is generic (k : natural); port (o : out natural); end;
architecture a of fp_leaf is
  signal hidden : natural := k;
begin
  hidden <= k + 10 after 5 ns;
  o <= hidden;
end;
entity fp_mon is generic (tag : string); port (i : in natural); end;
architecture a of fp_mon is
begin
  process begin
    wait for 3 ns;
    report tag & " early=" & integer'image(i);
    wait for 4 ns;
    report tag & " late=" & integer'image(i);
    wait;
  end process;
end;
entity fp_pair is end;
architecture a of fp_pair is
  signal x : natural;
begin
  l : entity work.fp_leaf generic map (k => 3) port map (o => x);
  m : entity work.fp_mon generic map (tag => "rel") port map (i => << signal l.hidden : natural >>);
  mu : entity work.fp_mon generic map (tag => "up") port map (i => << signal ^.p.l.hidden : natural >>);
end;
entity external_port_forms is end;
architecture a of external_port_forms is
begin
  g : for n in 1 to 2 generate
    l : entity work.fp_leaf generic map (k => n * 100) port map (o => open);
  end generate;
  p : entity work.fp_pair;
  mg : entity work.fp_mon generic map (tag => "gen2")
    port map (i => << signal .external_port_forms.g(2).l.hidden : natural >>);
  mk : entity work.fp_mon generic map (tag => "pkg")
    port map (i => << signal @work.fp_pkg.pk : natural >>);
  process begin
    work.fp_pkg.pk <= 7 after 5 ns;
    wait;
  end process;
end;
