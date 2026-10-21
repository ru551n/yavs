-- yavs-test: std=2008
-- expect: E1311 12:61 open sub-element before 2019
-- LRM 6.5.7.1 (2008): a formal associated individually cannot have open parts (sim1 and sim2 reject)
entity e is
end entity;
architecture a of e is
  signal v : bit_vector(15 downto 0);
  component c is
    port (lp : out bit_vector(31 downto 0));
  end component;
begin
  u : c port map (lp(15 downto 0) => v, lp(31 downto 16) => open);
end architecture;
