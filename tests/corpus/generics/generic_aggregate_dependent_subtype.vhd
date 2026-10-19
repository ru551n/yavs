-- difftest: top=generic_aggregate_dependent_subtype std=2008
-- An aggregate actual for a generic whose subtype uses an earlier generic takes its bounds
-- from that subtype with the earlier generic's actual (LRM 6.5.6.2, 14.3.2).
entity gadst_dev is
  generic (
    W   : natural range 1 to 64;
    DEF : bit_vector(W - 1 downto 0)
  );
  port (p : inout bit_vector(W - 1 downto 0) := DEF);
end entity;
architecture a of gadst_dev is
begin
  process begin
    report "W " & integer'image(W) & " p " & integer'image(p'length) & " " & to_string(p);
    wait;
  end process;
end architecture;

entity generic_aggregate_dependent_subtype is end entity;
architecture a of generic_aggregate_dependent_subtype is
  signal s1 : bit_vector(0 downto 0);
  signal s4 : bit_vector(3 downto 0);
begin
  i1 : entity work.gadst_dev generic map (W => 1, DEF => (others => '1')) port map (p => s1);
  i4 : entity work.gadst_dev generic map (W => 4, DEF => (others => '1')) port map (p => s4);
end architecture;
