-- difftest: top=external_alias_part_twice std=2008 stop=20ns ignore=order,delta
-- One architecture elaborated three times, with generics that select the branch holding aliases
-- and direct uses of parts of external names in one instance and not in another (LRM 8.7, 14.2):
-- the instance that elaborates the branch binds them, the one that does not never resolves them,
-- and one branch no instance selects (it names nothing, and one of its aliases is of a part).
-- ignore=order: reports of different processes in one cycle. ignore=delta: sim1 takes a delta cycle
-- for the port actual that is an external name or a part of one, yavs none (L49).
entity sub is end entity;
architecture a of sub is signal s : bit_vector(3 downto 0) := "0000"; signal t : bit := '0'; constant c : integer := 4; begin s <= "1010" after 1 ns; t <= '1' after 1 ns; end architecture;
entity probe is generic (tag : string); port (i : in bit); end;
architecture a of probe is begin process (i) begin report "probe " & tag & bit'image(i); end process; end;
entity inner is generic (V : natural := 1); end entity;
architecture a of inner is
begin
  g1 : if V = 1 generate
    alias w is <<signal .external_alias_part_twice.u.t : bit>>;
    alias b is <<signal .external_alias_part_twice.u.s : bit_vector(3 downto 0)>>(1);
    signal q : bit := <<signal .external_alias_part_twice.u.t : bit>>;
  begin
    process begin wait for 2 ns; report "inner V=1 " & bit'image(w) & bit'image(b) & integer'image(<<constant .external_alias_part_twice.u.c : integer>>); wait; end process;
    pr : entity work.probe generic map (tag => integer'image(V) & "pr ") port map (i => <<signal .external_alias_part_twice.u.t : bit>>);
    pr2 : entity work.probe generic map (tag => integer'image(V) & "pr2 ") port map (i => b);
  end generate;
  g2 : if V = 2 generate
    alias w is <<signal .external_alias_part_twice.u.t : bit>>;
    alias b is <<signal .external_alias_part_twice.u.s : bit_vector(3 downto 0)>>(1);
    signal q : bit := <<signal .external_alias_part_twice.u.t : bit>>;
  begin
    process begin wait on w, b; report "inner V=2 " & bit'image(w) & bit'image(b) & integer'image(<<constant .external_alias_part_twice.u.c : integer>>); wait; end process;
    pr : entity work.probe generic map (tag => integer'image(V) & "pr ") port map (i => <<signal .external_alias_part_twice.u.t : bit>>);
    pr2 : entity work.probe generic map (tag => integer'image(V) & "pr2 ") port map (i => b);
    pr3 : entity work.probe generic map (tag => integer'image(V) & "pr3 ") port map (i => <<signal .external_alias_part_twice.u.s : bit_vector(3 downto 0)>>(2));
  end generate;
  -- no instance selects this one
  g9 : if V = 9 generate
    alias b is <<signal .external_alias_part_twice.nowhere : bit_vector(3 downto 0)>>(1);
    alias w is <<signal .external_alias_part_twice.nowhere : bit>>;
  begin
    process (b, w) begin report "inner V=9 " & bit'image(w) & bit'image(b); end process;
  end generate;
end architecture;
entity external_alias_part_twice is end entity;
architecture a of external_alias_part_twice is
begin
  u : entity work.sub;
  i1 : entity work.inner generic map (V => 1);
  i2 : entity work.inner generic map (V => 2);
  i3 : entity work.inner generic map (V => 1);
  i4 : entity work.inner generic map (V => 3);
end architecture;
